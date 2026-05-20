// ministr.ai cloud — top-level composition.
//
// SOLID: this file orchestrates; each `lib/*.ts` builds one cohesive
// resource group. Resource dependencies flow one direction:
//   networking → registry / storage / insights → postgres → app
//              → role-assignments → domain.
//
// PHASE6 chunk 3 retired the indexer ACA Job. The serve pod's
// in-process WorkerLoop now drains `indexer_jobs` (see
// `ministr-cli/src/worker.rs`). The Job + its blob-data role + its
// jobs-operator role + the three `MINISTR_ACA_*` env vars that fed
// the deleted ARM trigger are all gone.

import * as pulumi from "@pulumi/pulumi";
import * as random from "@pulumi/random";

import { createNetworking } from "./lib/networking";
import { createRegistry } from "./lib/registry";
import { createStorage } from "./lib/storage";
import { createInsights } from "./lib/insights";
import { createApp } from "./lib/app";
import { bindCustomDomain } from "./lib/domain";
import { createPostgres } from "./lib/postgres";
import { createOpenAi } from "./lib/openai";
import {
  grantBlobDataContributor,
  grantCognitiveServicesUser,
} from "./lib/role-assignment";
import { named } from "./lib/naming";

const cfg = new pulumi.Config();
const imageTag = cfg.get("imageTag") ?? "latest";
const customDomain = cfg.get("customDomain") ?? "";
const appCpu = cfg.get("appCpu") ?? "0.5";
const appMemory = cfg.get("appMemory") ?? "1Gi";
// Colon-separated paths the container should index. Defaults to a
// pod-local path under $HOME (= /data per the image's useradd). The
// app's auto-register on boot creates the dir if missing; the demo's
// clone endpoint writes clones here.
const corpusPaths = cfg.get("corpusPaths") ?? "/data/corpus";
const webhookSecret = cfg.getSecret("githubWebhookSecret");

// Postgres is now the default backend for OAuth + tenancy + usage
// events. Toggle off by `pulumi config set enablePostgres false` for
// a strictly local-dev preview; production deploys always run with
// Postgres on.
const enablePostgres = cfg.getBoolean("enablePostgres") ?? true;
const pgAdminLogin = cfg.get("pgAdminLogin") ?? "ministradmin";
// PHASE6 chunk 4a — Azure OpenAI is the cloud worker's embedder.
// Toggle off (`enableOpenAi=false`) to provision the rest of the
// stack without the OpenAI account — the serve pod will then run
// without `MINISTR_EMBEDDER_KIND=openai` and fall back to local
// fastembed (which OOMs on 2 GiB pods, see PHASE6.md — so this is
// only useful for the bootstrap step before the OpenAI capacity has
// been approved in the subscription).
const enableOpenAi = cfg.getBoolean("enableOpenAi") ?? true;
const openaiLocation = cfg.get("openaiLocation");
// Some subscriptions are restricted from provisioning Postgres Flex
// Burstable in certain regions (e.g. eastus). Override with
// `pulumi config set pgLocation westus2`.
const pgLocation = cfg.get("pgLocation");
// Auto-generate the admin password if the operator hasn't pinned one.
// `random.RandomPassword` is stateful — Pulumi persists the generated
// value in the stack so subsequent runs reuse the same password.
const pgAdminPasswordCfg = cfg.getSecret("pgAdminPassword");
const pgAutoPassword = enablePostgres && !pgAdminPasswordCfg
  ? new random.RandomPassword(named("pg-admin-pw"), {
      length: 32,
      special: true,
      // Azure Postgres rejects these characters in the admin password.
      overrideSpecial: "_-.~",
    })
  : undefined;
const pgAdminPassword =
  pgAdminPasswordCfg ?? pgAutoPassword?.result;

const net = createNetworking();
const registry = createRegistry({ rg: net.rg });
const storage = createStorage({ rg: net.rg, env: net.env });
const insights = createInsights({ rg: net.rg, workspace: net.workspace });

const postgres =
  enablePostgres && pgAdminPassword
    ? createPostgres({
        rg: net.rg,
        adminLogin: pgAdminLogin,
        adminPassword: pulumi.secret(pgAdminPassword),
        pgLocation,
      })
    : undefined;

const openai = enableOpenAi
  ? createOpenAi({
      rg: net.rg,
      openaiLocation,
    })
  : undefined;

// Predict the ACA-assigned FQDN from the env's default domain + the
// container app name so we can feed it into the app's env vars at plan
// time without a two-step apply.
const predictedHost = pulumi.interpolate`${named("app")}.${net.env.defaultDomain}`;
const publicHost: pulumi.Input<string> = customDomain || predictedHost;
const publicUrl = pulumi.interpolate`https://${publicHost}`;

const queryApp = createApp({
  rg: net.rg,
  env: net.env,
  registry,
  storage,
  insights,
  imageTag,
  cpu: appCpu,
  memory: appMemory,
  webhookSecret,
  corpusPaths,
  publicUrl,
  publicHost,
  pgConnectionString: postgres?.pgConnectionString,
  openaiEndpoint: openai?.endpoint,
  openaiDeployment: openai?.deploymentName,
});

// Grant the app's managed identity read+write on the corpora blob
// container. Without this the Rust ManagedIdentityCredential chain
// gets a token but every blob op returns 403.
grantBlobDataContributor({
  name: named("app-blob-rw"),
  storageAccount: storage.account,
  principalId: queryApp.principalId,
});

// PHASE6 chunk 4a — grant the app's MI Cognitive Services User on the
// OpenAI account. The worker mints a Bearer via the same MI; without
// this role grant, POST /embeddings returns 403 and ingestion stalls.
if (openai) {
  grantCognitiveServicesUser({
    name: named("app-openai-user"),
    accountId: openai.accountId,
    principalId: queryApp.principalId,
  });
}

const domainBinding = customDomain
  ? bindCustomDomain({
      rg: net.rg,
      env: net.env,
      containerApp: queryApp.containerApp,
      domain: customDomain,
    })
  : undefined;

export const resourceGroup = net.rg.name;
export const registryServer = registry.loginServer;
export const storageAccount = storage.accountName;
export const blobContainer = storage.blobContainerName;
export const appFqdn = queryApp.fqdn;
// Container App resource name — used by just recipes (azure-logs,
// azure-restart-app, azure-rbac-reconcile) to look up the live
// resource without hardcoding the `ministrv2` project prefix.
export const appName = queryApp.containerApp.name;
export const customDomainConfigured = customDomain || "(none)";
export const customDomainCertId = domainBinding?.apply((d) => d.certId);
export const publicBaseUrl = publicUrl;
export const pgHost = postgres?.host;
export const pgConnectionString = postgres?.pgConnectionString;
// PHASE6 chunk 4a — surface the OpenAI endpoint + deployment so the
// operator can sanity-check via `pulumi stack output` after a deploy.
export const openaiEndpoint = openai?.endpoint;
export const openaiDeployment = openai?.deploymentName;
