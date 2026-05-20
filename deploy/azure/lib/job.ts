// Indexer ACA Job — current trigger: KEDA postgresql scaler (PHASE4
// chunk 1). Slated for replacement: PHASE5 chunk 1 swaps the primary
// trigger to a direct ARM `POST /jobs/{name}/start` REST call from
// the serve pod immediately after enqueue, and degrades this KEDA
// scaler to a slow-poll (5 min) safety net for missed triggers.
//
// What's here right now:
//   - `triggerType: "Event"` with a KEDA postgresql scaler against
//     `indexer_jobs` (SELECT count(*) WHERE status='pending', polled
//     every 5s). Whenever count ≥ targetQueryValue (1), ACA starts
//     one replica running the `indexer-worker` entrypoint — it
//     claims one row, runs ingestion, uploads, exits. Idle queue ⇒
//     scaler reports 0 ⇒ minExecutions=0 ⇒ $0/mo on the job side
//     (vs ~$15/mo of empty cron ticks under PHASE3 chunk 6).
//
//   - Auth: the scaler reads the same `pg-url` secret the worker
//     uses, injected as the KEDA `connection` triggerParameter via
//     the standard ScaleRuleAuth mapping. No separate Postgres
//     principal needed.
//
//   - Blob auth via SystemAssigned managed identity (mirrors the
//     query app); `grantBlobDataContributor` in index.ts scopes the
//     role.
//
// Why this is being retired in PHASE5 chunk 1: ACA's `triggerType:
// "Event"` is marketing — KEDA is still polling Postgres every 5s.
// Real event-driven means the producer (serve pod) tells ACA to
// start a replica directly. The original PHASE3 chunk 6 "option B"
// designed exactly that and was deferred for one RBAC role
// assignment on the serve pod MI — a bad trade we are now undoing.
// See `deploy/azure/PHASE5.md` and the `feedback-no-rbac-deferral`
// memory for the principle.
//
// Sized for the streaming pipeline (PHASE4 chunk 4 + 5):
// 2 vCPU / 4 GiB by default. Override via Pulumi.prod.yaml.

import * as pulumi from "@pulumi/pulumi";
import * as app from "@pulumi/azure-native/app";
import * as resources from "@pulumi/azure-native/resources";
import * as types from "@pulumi/azure-native/types/input";

import { location, named } from "./naming";
import { RegistryArtifact } from "./registry";
import { StorageArtifact } from "./storage";

export interface JobArtifact {
  job: app.Job;
  name: pulumi.Output<string>;
  /** Managed identity principal id — feed into role assignments. */
  principalId: pulumi.Output<string>;
}

export interface JobInputs {
  rg: resources.ResourceGroup;
  env: app.ManagedEnvironment;
  registry: RegistryArtifact;
  storage: StorageArtifact;
  imageTag: string;
  cpu: string;
  memory: string;
  /**
   * Cloud Postgres connection string (libpq URL). Required by the
   * `indexer-worker` to claim jobs and update progress. When absent,
   * the job is provisioned without the secret env and the worker
   * exits with an error on first tick — deployments that opt out
   * of Postgres also opt out of the worker.
   */
  pgConnectionString?: pulumi.Input<string>;
}

export function createIndexerJob(inputs: JobInputs): JobArtifact {
  const { rg, env, registry, storage, imageTag, cpu, memory, pgConnectionString } = inputs;

  const imageRef = pulumi.interpolate`${registry.loginServer}/ministr:${imageTag}`;

  const secretsList: pulumi.Input<types.app.SecretArgs>[] = [
    { name: "registry-password", value: registry.adminPassword },
  ];
  if (pgConnectionString) {
    secretsList.push({ name: "pg-url", value: pgConnectionString });
  }

  const baseEnv: pulumi.Input<types.app.EnvironmentVarArgs>[] = [
    // Queue-driven worker entrypoint (added in PHASE3 chunk 6, still
    // the right shape under PHASE4's event trigger — the replica is
    // single-shot either way; KEDA just decides _when_ to start it).
    { name: "ENTRYPOINT_MODE", value: "indexer-worker" },
    // Blob backend for both the corpora.json restore and per-corpus
    // bundle uploads. Managed identity (see grantBlobDataContributor
    // in index.ts) authorises the actual ops; the env just selects
    // the backend kind and points at the account/container.
    { name: "MINISTR_BLOB_STORE_KIND", value: "azure" },
    { name: "MINISTR_BLOB_AZURE_ACCOUNT", value: storage.accountName },
    { name: "MINISTR_BLOB_AZURE_CONTAINER", value: storage.blobContainerName },
    // PHASE3 fix E — anyhow-sized ingest peaks at ~7.5 GiB rss with
    // the full-fat `all-MiniLM-L6-v2` ONNX model on 4 vCPU / 8 GiB,
    // which OOM-kills the container mid-embedding (exit 137).
    // `MINISTR_PREFER_QUANTIZED=1` swaps in `all-MiniLM-L6-v2-q`, an
    // INT8-quantised variant of the same model — 2-4× smaller memory
    // footprint with equivalent retrieval quality on code corpora.
    // No model-download or schema change required; the embedder picks
    // it up from the fastembed cache. Drop this env once the worker
    // is sized larger (or option-B Azure-REST trigger lands).
    { name: "MINISTR_PREFER_QUANTIZED", value: "1" },
    { name: "RUST_LOG", value: "info,ministr=debug" },
  ];
  if (pgConnectionString) {
    baseEnv.push({ name: "MINISTR_PG_URL", secretRef: "pg-url" });
  }

  const job = new app.Job(named("indexer"), {
    resourceGroupName: rg.name,
    jobName: named("indexer"),
    location,
    environmentId: env.id,
    // SystemAssigned MI authenticates blob ops the same way the query
    // app does. The role assignment in index.ts scopes blob-data-
    // contributor on this principal.
    identity: { type: "SystemAssigned" },
    configuration: {
      // PHASE5 chunk 1 — KEDA postgres-poll demoted to safety net.
      //
      // The fast path now is the serve pod calling ARM
      // `POST /jobs/{name}/start` directly after enqueue (see
      // `ministr-cloud::AcaJobStartTrigger`). This scaler keeps the
      // same `triggerType: "Event"` + postgres rule but at a 5-minute
      // polling interval — it is the floor that catches rows the ARM
      // call missed (ARM 5xx, transient network failure on the serve
      // pod, etc). `maxExecutions: 1` keeps it single-replica today
      // (single-tenant cloud); raise once the worker concurrency
      // backlog item is picked up.
      //
      // Cost effect of the bump: 12 KEDA queries/hour instead of 720
      // — empty-tick Postgres load drops by ~98%. The bump landed in
      // PHASE5 chunk 1 alongside the ARM trigger.
      triggerType: "Event",
      replicaTimeout: 3600, // 1h hard cap (matches ACA Jobs default ceiling).
      replicaRetryLimit: 0,
      eventTriggerConfig: {
        replicaCompletionCount: 1,
        parallelism: 1,
        scale: {
          minExecutions: 0,
          maxExecutions: 1,
          // PHASE5 chunk 1 — 300s (5min) is the safety-net cadence;
          // ARM jobs/start from the serve pod is the fast path.
          pollingInterval: 300,
          rules: [
            {
              name: "pg-pending",
              type: "postgresql",
              // KEDA postgresql scaler metadata. `targetQueryValue` is
              // the threshold above which the scaler creates an
              // execution; with `1` the scaler fires whenever
              // `count(*) >= 1`. The query MUST return a single
              // numeric column.
              metadata: {
                query:
                  "SELECT count(*)::int FROM indexer_jobs WHERE status='pending'",
                targetQueryValue: "1",
              },
              // Map the existing `pg-url` secret onto KEDA's
              // `connection` parameter — the scaler reads the
              // connection string from there. The worker container
              // still reads the same secret via `MINISTR_PG_URL`
              // env (below), so there's only one secret to rotate.
              auth: [{ secretRef: "pg-url", triggerParameter: "connection" }],
            },
          ],
        },
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
          name: "indexer",
          image: imageRef,
          resources: { cpu: Number(cpu), memory },
          env: baseEnv,
        },
      ],
    },
  });

  const principalId = job.identity.apply(
    (i) => i?.principalId ?? "",
  );

  return { job, name: job.name, principalId };
}
