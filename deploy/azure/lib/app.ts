// Query Container App.
//
// Sizing: 0.5 vCPU / 1 GiB, minReplicas=1, maxReplicas=1 — always-warm,
// no cold starts, no concurrent-writer SQLite hazards.
// Mounts the shared Azure Files share at /data so OAuth + admin SQLite
// state persists across pod restarts.

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
  } = inputs;

  const imageRef = pulumi.interpolate`${registry.loginServer}/ministr:${imageTag}`;

  // Secrets surface to the container as ContainerApp.secrets[] entries that
  // env vars and `registries[].passwordSecretRef` reference by name.
  const baseSecrets: types.app.SecretArgs[] = [
    { name: "registry-password", value: registry.adminPassword },
    { name: "appinsights-connection-string", value: insights.connectionString },
  ];
  const secrets: pulumi.Input<types.app.SecretArgs>[] = webhookSecret
    ? [...baseSecrets, { name: "github-webhook-secret", value: webhookSecret }]
    : baseSecrets;

  const baseEnv: types.app.EnvironmentVarArgs[] = [
    { name: "MINISTR_CLOUD_DATA_DIR", value: "/data" },
    { name: "MINISTR_CORPUS_PATHS", value: corpusPaths },
    // Enables OAuth on the public deployment — the entrypoint passes
    // `--oauth --oauth-issuer $MINISTR_OAUTH_ISSUER` when this is set.
    // Without it, every endpoint we just mounted (including the new
    // `/api/v1/corpora/*` write routes) would be open to the internet.
    { name: "MINISTR_OAUTH_ISSUER", value: "https://mcp.ministr.ai" },
    {
      name: "APPLICATIONINSIGHTS_CONNECTION_STRING",
      secretRef: "appinsights-connection-string",
    },
    { name: "RUST_LOG", value: "info,ministr=debug" },
  ];
  const envVars: pulumi.Input<types.app.EnvironmentVarArgs>[] = webhookSecret
    ? [
        ...baseEnv,
        { name: "MINISTR_GITHUB_WEBHOOK_SECRET", secretRef: "github-webhook-secret" },
      ]
    : baseEnv;

  const containerApp = new app.ContainerApp(named("app"), {
    resourceGroupName: rg.name,
    containerAppName: named("app"),
    location,
    managedEnvironmentId: env.id,
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
      secrets,
    },
    template: {
      containers: [
        {
          name: "ministr",
          image: imageRef,
          resources: { cpu: Number(cpu), memory },
          env: envVars,
          volumeMounts: [{ volumeName: "data", mountPath: "/data" }],
          // Probes intentionally omitted during placeholder bootstrap — they
          // require /healthz on 8080 and would block ACA from accepting the
          // first revision. Re-add once the real ministr image is in ACR.
        },
      ],
      volumes: [
        {
          name: "data",
          storageType: "AzureFile",
          storageName: storage.storageName,
        },
      ],
      scale: { minReplicas: 1, maxReplicas: 1 },
    },
  });

  return {
    containerApp,
    fqdn: containerApp.configuration.apply(
      (c) => c?.ingress?.fqdn ?? "",
    ) as pulumi.Output<string>,
  };
}
