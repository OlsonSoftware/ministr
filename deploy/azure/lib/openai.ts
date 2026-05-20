// Azure OpenAI resource — the cloud worker's embedder home.
//
// PHASE6 chunk 4a — provisions the Cognitive Services account
// (`kind: "OpenAI"`) and a model deployment for
// `text-embedding-3-small`. The deployment name is what the Rust
// worker passes as `MINISTR_AZURE_OPENAI_DEPLOYMENT`; the resource's
// `*.openai.azure.com` endpoint is `MINISTR_AZURE_OPENAI_ENDPOINT`.
//
// Auth path: the serve pod uses its managed identity to mint a Bearer
// against `https://cognitiveservices.azure.com`. No API key needs to
// land in env vars or secrets — the role assignment in
// `lib/role-assignment.ts::grantCognitiveServicesUser` is the only
// thing standing between the pod and a 403.
//
// Pricing note (May 2026): S0 is the only SKU for net-new accounts.
// Embedding calls bill at $0.02/1M tokens for `text-embedding-3-small`;
// at typical Pro-tier usage this is pennies/user/month and well inside
// the project's <$200/mo guard-rail.

import * as pulumi from "@pulumi/pulumi";
import * as resources from "@pulumi/azure-native/resources";
import * as cognitive from "@pulumi/azure-native/cognitiveservices";

import { location, named } from "./naming";

export interface OpenAiArtifact {
  account: cognitive.Account;
  deployment: cognitive.Deployment;
  /** `https://<account>.openai.azure.com` — the base URL the Rust worker
   * passes as `MINISTR_AZURE_OPENAI_ENDPOINT`. The Rust embedder
   * appends `/openai/deployments/{deployment}/embeddings?...`. */
  endpoint: pulumi.Output<string>;
  /** Account principal-scope id, suitable for role assignments. */
  accountId: pulumi.Output<string>;
  /** Operator-friendly deployment name passed to the Rust worker as
   * `MINISTR_AZURE_OPENAI_DEPLOYMENT`. */
  deploymentName: pulumi.Output<string>;
}

export interface OpenAiInputs {
  rg: resources.ResourceGroup;
  /**
   * Azure region for the Cognitive Services account. Defaults to the
   * stack's global `location`. Embedding model availability varies by
   * region — `eastus`, `westus`, `westeurope`, and `southcentralus` are
   * the safe choices for `text-embedding-3-small` as of mid-2026.
   * Override via `pulumi config set openaiLocation ...`.
   */
  openaiLocation?: string;
  /**
   * Model name to deploy. Defaults to `text-embedding-3-small`.
   * Operators can swap to `text-embedding-3-large` for full quality
   * at $0.13/1M tokens, but the HNSW indexes built against `small`
   * with 384 dims won't be query-compatible with `large`'s 3072 dims —
   * pin this and don't change it for a live deployment.
   */
  modelName?: string;
  /**
   * Model version. The Azure catalog tags `text-embedding-3-small`'s
   * latest stable as `"1"`. Pulumi only re-deploys on version change.
   */
  modelVersion?: string;
  /**
   * Throughput allocation — tokens-per-minute / 1000. `1` = 1K TPM.
   * The S0 account-level cap is ~350K TPM for embeddings in 2026;
   * 10 = 10K TPM is generous for a single Pro-tier worker and bills
   * the same per-token whether you reserve more or less. Bump if you
   * see HTTP 429s in the worker logs.
   */
  capacity?: number;
}

export function createOpenAi({
  rg,
  openaiLocation,
  modelName,
  modelVersion,
  capacity,
}: OpenAiInputs): OpenAiArtifact {
  const accountLocation = openaiLocation ?? location;
  const modelDeploymentName = modelName ?? "text-embedding-3-small";
  const resolvedModelVersion = modelVersion ?? "1";
  const resolvedCapacity = capacity ?? 10;

  const account = new cognitive.Account(named("openai"), {
    resourceGroupName: rg.name,
    accountName: named("openai"),
    location: accountLocation,
    // Microsoft Entra (a.k.a. AAD) only — no API-key auth path. The
    // Rust embedder's `OpenAiAuth::ManagedIdentity` variant mints
    // tokens via the pod's MI; setting `disableLocalAuth: true` here
    // is the belt-and-suspenders side of that contract.
    kind: "OpenAI",
    sku: { name: "S0" },
    identity: { type: "SystemAssigned" },
    properties: {
      // Keep API-key auth ENABLED for now — the Rust embedder supports
      // both paths and the operator may want to bootstrap with an API
      // key in env before MI propagation lands. Once the live demo
      // confirms MI auth round-trips end-to-end (PHASE6 chunk 4b),
      // flip `disableLocalAuth: true` here.
      disableLocalAuth: false,
      publicNetworkAccess: cognitive.PublicNetworkAccess.Enabled,
      // Custom subdomain is required for any Cognitive Services
      // account that supports Entra auth. Without it, MI token
      // requests fail with `InvalidAudience`. The subdomain becomes
      // the `<name>.openai.azure.com` endpoint hostname.
      customSubDomainName: named("openai"),
    },
  });

  // Deploy the embedding model. The deployment name (NOT the model
  // name) is what gets baked into the worker's request URL.
  const deployment = new cognitive.Deployment(
    named("openai-embed"),
    {
      resourceGroupName: rg.name,
      accountName: account.name,
      deploymentName: modelDeploymentName,
      sku: {
        // Standard = pay-per-use. The alternative (`GlobalStandard`)
        // routes across Azure's global capacity for higher throughput
        // but adds per-region cost. Stick with `Standard` until the
        // single-region cap (350K TPM) is the bottleneck.
        name: "Standard",
        capacity: resolvedCapacity,
      },
      properties: {
        model: {
          format: "OpenAI",
          name: modelDeploymentName,
          version: resolvedModelVersion,
        },
        versionUpgradeOption:
          cognitive.DeploymentModelVersionUpgradeOption.OnceCurrentVersionExpired,
      },
    },
    { dependsOn: [account] },
  );

  const endpoint = pulumi.interpolate`https://${account.properties.apply(
    (p) => p?.customSubDomainName ?? "",
  )}.openai.azure.com`;

  return {
    account,
    deployment,
    endpoint,
    accountId: account.id,
    deploymentName: deployment.name,
  };
}
