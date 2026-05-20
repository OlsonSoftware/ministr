// ministr.ai cloud — top-level composition.
//
// SOLID: this file orchestrates; each `lib/*.ts` builds one cohesive
// resource group. Resource dependencies flow one direction:
//   networking → registry / storage / insights → postgres → app / job
//              → role-assignments → domain.

import * as pulumi from "@pulumi/pulumi";
import * as random from "@pulumi/random";

import { createNetworking } from "./lib/networking";
import { createRegistry } from "./lib/registry";
import { createStorage } from "./lib/storage";
import { createInsights } from "./lib/insights";
import { createApp } from "./lib/app";
import { createIndexerJob } from "./lib/job";
import { bindCustomDomain } from "./lib/domain";
import { createPostgres } from "./lib/postgres";
import { grantBlobDataContributor } from "./lib/role-assignment";
import { grantJobsOperator } from "./lib/job-start-role";
import * as authorization from "@pulumi/azure-native/authorization";
import { named } from "./lib/naming";

const cfg = new pulumi.Config();
const imageTag = cfg.get("imageTag") ?? "latest";
const customDomain = cfg.get("customDomain") ?? "";
const appCpu = cfg.get("appCpu") ?? "0.5";
const appMemory = cfg.get("appMemory") ?? "1Gi";
const jobCpu = cfg.get("jobCpu") ?? "4";
const jobMemory = cfg.get("jobMemory") ?? "8Gi";
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

// Predict the ACA-assigned FQDN from the env's default domain + the
// container app name so we can feed it into the app's env vars at plan
// time without a two-step apply.
const predictedHost = pulumi.interpolate`${named("app")}.${net.env.defaultDomain}`;
const publicHost: pulumi.Input<string> = customDomain || predictedHost;
const publicUrl = pulumi.interpolate`https://${publicHost}`;

// Build the indexer Job first so its name + RG can be threaded into the
// serve pod's env (PHASE5 chunk 1's fast-path config). Pulumi resolves
// the Output dependencies regardless of code order, but reading-order
// matches eval-order here.
const indexer = createIndexerJob({
  rg: net.rg,
  env: net.env,
  registry,
  storage,
  imageTag,
  cpu: jobCpu,
  memory: jobMemory,
  pgConnectionString: postgres?.pgConnectionString,
});

// PHASE3 chunk 6 — the indexer worker uses ManagedIdentityCredential
// for blob ops (download + upload), so its MI needs blob-data access
// scoped to the corpora storage account. Mirrors the queryApp grant
// below; both principals get the same role on the same scope.
grantBlobDataContributor({
  name: named("indexer-blob-rw"),
  storageAccount: storage.account,
  principalId: indexer.principalId,
});

// PHASE5 chunk 1 — sub id sourced from the current Azure session;
// fed both to grantJobsOperator (scope construction) and the serve pod
// env (URL construction). Output<string> threads through both call
// sites unmodified.
const subscriptionId =
  authorization.getClientConfigOutput().subscriptionId;

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
  // PHASE5 chunk 1 — fast-path ARM trigger config. The Rust trigger
  // (`AcaJobStartTrigger::from_env`) requires all three to resolve;
  // any one missing falls back to KEDA-only.
  acaSubscriptionId: subscriptionId,
  acaResourceGroup: net.rg.name,
  acaIndexerJobName: indexer.name,
});

// Grant the app's managed identity read+write on the corpora blob
// container. Without this the Rust ManagedIdentityCredential chain
// gets a token but every blob op returns 403.
grantBlobDataContributor({
  name: named("app-blob-rw"),
  storageAccount: storage.account,
  principalId: queryApp.principalId,
});

// PHASE5 chunk 1 — grant the serve pod's MI `Container Apps Jobs
// Operator` scoped to the indexer Job so its ARM POST .../start
// succeeds. This is the role PHASE3 chunk 6 declined to add. See
// `lib/job-start-role.ts` + `PHASE5.md` + the
// `feedback-no-rbac-deferral` memory.
grantJobsOperator({
  name: named("app-jobs-start"),
  indexerJob: indexer.job,
  principalId: queryApp.principalId,
});

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
export const indexerJobName = indexer.name;
export const customDomainConfigured = customDomain || "(none)";
export const customDomainCertId = domainBinding?.apply((d) => d.certId);
export const publicBaseUrl = publicUrl;
export const pgHost = postgres?.host;
export const pgConnectionString = postgres?.pgConnectionString;
