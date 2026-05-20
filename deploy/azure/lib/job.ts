// Indexer ACA Job — PHASE4 chunk 1.
//
// Event-driven trigger (KEDA postgresql scaler): the job's KEDA
// postgresql scaler polls the cloud Postgres `indexer_jobs` queue
// every 5s; whenever the SELECT count of `status='pending'` rows
// exceeds `targetQueryValue`, ACA starts one replica. The replica
// runs the `indexer-worker` entrypoint — claims one row, runs
// ingestion, uploads the bundle, exits. With the queue drained the
// scaler reports 0 and replicas stay at minExecutions=0, so an idle
// cluster costs ~$0/mo on the job side (was ~$15/mo of empty cron
// ticks under PHASE3 chunk 6).
//
// Auth: the scaler reads the same `pg-url` secret the worker uses,
// injected as the KEDA `connection` triggerParameter via the standard
// ScaleRuleAuth mapping. No separate Postgres principal needed.
//
// Sized for headroom under the monolithic ingest pipeline (PHASE4
// chunk 4 will switch to streaming + downsize to 4 GiB / 2 vCPU in
// chunk 5). Blob auth via SystemAssigned managed identity (mirrors
// the query app); `grantBlobDataContributor` in index.ts scopes the
// role. PHASE3 chunk 6 option B (serve-pod triggers via the Azure
// REST API) is now superseded by KEDA and removed from the backlog.

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
      // PHASE4 chunk 1 — KEDA event-driven trigger. The postgresql
      // scaler polls `indexer_jobs` every 5s; replicas spin up only
      // when `status='pending'` rows exist. Idle queue ⇒ 0 replicas
      // ⇒ $0/h. `maxExecutions: 1` keeps it single-replica today
      // (single-tenant cloud); raise once the worker concurrency
      // backlog item is picked up.
      triggerType: "Event",
      replicaTimeout: 3600, // 1h hard cap (matches ACA Jobs default ceiling).
      replicaRetryLimit: 0,
      eventTriggerConfig: {
        replicaCompletionCount: 1,
        parallelism: 1,
        scale: {
          minExecutions: 0,
          maxExecutions: 1,
          pollingInterval: 5,
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
