// Query Container App.
//
// Sizing: 0.5 vCPU / 1 GiB, minReplicas=1, maxReplicas=1 — always-warm,
// no cold starts.
//
// Architecture (Option B):
//   - SystemAssigned managed identity, granted `Storage Blob Data
//     Contributor` on the storage account (see lib/role-assignment.ts)
//     so the container can read/write the corpora blob container with
//     no shared key.
//   - `MINISTR_PG_URL` (secret env) → Postgres Flex for OAuth +
//     tenancy + usage events.
//   - `MINISTR_BLOB_STORE_KIND=azure` + account + container env →
//     `CorpusBlobStore` durable HNSW bundles.
//   - **No volume mount.** `$HOME=/data` (from the image's `useradd`)
//     is the pod-local writable filesystem; the daemon's working
//     SQLite + HNSW live there and warm-restart from blob at boot.

import * as pulumi from "@pulumi/pulumi";
import * as app from "@pulumi/azure-native/app";
import * as resources from "@pulumi/azure-native/resources";
import * as types from "@pulumi/azure-native/types/input";

import { location, named } from "./naming";
import { RegistryArtifact } from "./registry";
import { StorageArtifact } from "./storage";
import { InsightsArtifact } from "./insights";

export interface AppArtifact {
  containerApp: app.ContainerApp;
  fqdn: pulumi.Output<string>;
  /** Managed identity principal id — feed into role assignments. */
  principalId: pulumi.Output<string>;
}

export interface AppInputs {
  rg: resources.ResourceGroup;
  env: app.ManagedEnvironment;
  registry: RegistryArtifact;
  storage: StorageArtifact;
  insights: InsightsArtifact;
  imageTag: string;
  cpu: string;
  memory: string;
  webhookSecret?: pulumi.Output<string>;
  corpusPaths: string;
  // Public URL users hit. Used for OAuth issuer + cloud base URL.
  publicUrl: pulumi.Input<string>;
  // Bare host for MINISTR_ALLOWED_HOSTS.
  publicHost: pulumi.Input<string>;
  // Postgres connection URI (secret). When set, OAuth + tenancy live in
  // Postgres and the F1.3+ routes (billing, Stripe, GitHub IdP, Atlas)
  // auto-mount. When unset the cloud falls back to SQLite + skips those
  // routes — fine for local dev, never for prod.
  pgConnectionString?: pulumi.Input<string>;
  // PHASE6 chunk 4a — Azure OpenAI embedder.
  //
  // When both `openaiEndpoint` and `openaiDeployment` resolve, the
  // serve pod gets `MINISTR_EMBEDDER_KIND=openai` and the two endpoint
  // env vars. Without them the pod falls back to the local fastembed
  // path — fine for local dev, NOT for the cloud (where the local
  // path OOMs at 3.6 GB ONNX activation memory).
  //
  // No API key here — the worker's `OpenAiAuth::ManagedIdentity` path
  // mints a Bearer via the pod's MI. The Cognitive Services User role
  // grant in index.ts is what gates 200 vs 403.
  openaiEndpoint?: pulumi.Input<string>;
  openaiDeployment?: pulumi.Input<string>;
}

export function createApp(inputs: AppInputs): AppArtifact {
  const {
    rg,
    env,
    registry,
    storage,
    insights,
    imageTag,
    cpu,
    memory,
    webhookSecret,
    corpusPaths,
    publicUrl,
    publicHost,
    pgConnectionString,
    openaiEndpoint,
    openaiDeployment,
  } = inputs;

  const imageRef = pulumi.interpolate`${registry.loginServer}/ministr:${imageTag}`;

  // Secrets surface to the container as ContainerApp.secrets[] entries that
  // env vars and `registries[].passwordSecretRef` reference by name.
  const secretsList: pulumi.Input<types.app.SecretArgs>[] = [
    { name: "registry-password", value: registry.adminPassword },
    { name: "appinsights-connection-string", value: insights.connectionString },
  ];
  if (webhookSecret) {
    secretsList.push({ name: "github-webhook-secret", value: webhookSecret });
  }
  if (pgConnectionString) {
    secretsList.push({ name: "pg-url", value: pgConnectionString });
  }

  const baseEnv: pulumi.Input<types.app.EnvironmentVarArgs>[] = [
    // Intentionally NOT setting MINISTR_CORPUS_PATHS — that triggers a
    // boot-time auto-register of the path, which crashes on an empty
    // dir (HNSW persist with 0 points fails to rename) and leaves the
    // corpus in a half-state that breaks later POST registrations of
    // the same path. The cloud starts with zero corpora; clients POST
    // /api/v1/corpora to register theirs.
    //
    // OAuth + base URL: must equal the URL clients hit, since OAuth
    // Discovery emits absolute URLs built from the issuer.
    { name: "MINISTR_OAUTH_ISSUER", value: publicUrl },
    { name: "MINISTR_CLOUD_BASE_URL", value: publicUrl },
    // Streamable HTTP transport rejects non-allowlisted Host headers.
    { name: "MINISTR_ALLOWED_HOSTS", value: publicHost },
    // Blob backend for durable HNSW bundles. CorpusBlobStore uses
    // DeveloperToolsCredential locally and ManagedIdentityCredential
    // in-pod (auto-detected by the azure_identity chain). The role
    // assignment in index.ts grants this identity blob-data access.
    { name: "MINISTR_BLOB_STORE_KIND", value: "azure" },
    { name: "MINISTR_BLOB_AZURE_ACCOUNT", value: storage.accountName },
    { name: "MINISTR_BLOB_AZURE_CONTAINER", value: storage.blobContainerName },
    {
      name: "APPLICATIONINSIGHTS_CONNECTION_STRING",
      secretRef: "appinsights-connection-string",
    },
    { name: "RUST_LOG", value: "info,ministr=debug" },
  ];
  if (webhookSecret) {
    baseEnv.push({
      name: "MINISTR_GITHUB_WEBHOOK_SECRET",
      secretRef: "github-webhook-secret",
    });
  }
  if (pgConnectionString) {
    baseEnv.push({ name: "MINISTR_PG_URL", secretRef: "pg-url" });
  }
  // PHASE6 chunk 4a — Azure OpenAI embedder env. Both endpoint and
  // deployment must resolve together; either missing falls back to
  // local fastembed (defeats the purpose on a 2 GiB pod, but at least
  // the binary boots so the operator can fix the gap).
  if (openaiEndpoint && openaiDeployment) {
    baseEnv.push({ name: "MINISTR_EMBEDDER_KIND", value: "openai" });
    baseEnv.push({ name: "MINISTR_AZURE_OPENAI_ENDPOINT", value: openaiEndpoint });
    baseEnv.push({
      name: "MINISTR_AZURE_OPENAI_DEPLOYMENT",
      value: openaiDeployment,
    });
  }

  const containerApp = new app.ContainerApp(named("app"), {
    resourceGroupName: rg.name,
    containerAppName: named("app"),
    location,
    managedEnvironmentId: env.id,
    identity: { type: "SystemAssigned" },
    configuration: {
      activeRevisionsMode: "Single",
      ingress: {
        external: true,
        targetPort: 8080,
        transport: "auto",
        allowInsecure: false,
      },
      registries: [
        {
          server: registry.loginServer,
          username: registry.adminUsername,
          passwordSecretRef: "registry-password",
        },
      ],
      secrets: secretsList,
    },
    template: {
      containers: [
        {
          name: "ministr",
          image: imageRef,
          resources: { cpu: Number(cpu), memory },
          env: baseEnv,
          // No volumeMounts — $HOME=/data is the pod-local writable FS
          // (from the image's useradd). Blob is the durable backing
          // store; local /data is the working cache.
        },
      ],
      scale: { minReplicas: 1, maxReplicas: 1 },
    },
  });

  // `identity.principalId` is populated by the platform after creation;
  // the apply() unwraps the Output<{...}|undefined> for downstream role
  // assignments.
  const principalId = containerApp.identity.apply(
    (i) => i?.principalId ?? "",
  ) as pulumi.Output<string>;

  return {
    containerApp,
    fqdn: containerApp.configuration.apply(
      (c) => c?.ingress?.fqdn ?? "",
    ) as pulumi.Output<string>,
    principalId,
  };
}
