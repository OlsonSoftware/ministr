// Indexer ACA Job.
//
// Manually triggered (via Azure REST API or the App's `/reindex` endpoint
// once it can call into the Container Apps Jobs plane in a v2). Sized
// big (4 vCPU / 8 GiB) so a reindex completes fast; pays per second
// while running, $0 when idle.
//
// Mounts the same Azure Files share at /data and writes corpora into
// `/data/corpora/<id>/` for the App to read.

import * as pulumi from "@pulumi/pulumi";
import * as app from "@pulumi/azure-native/app";
import * as resources from "@pulumi/azure-native/resources";

import { location, named } from "./naming";
import { RegistryArtifact } from "./registry";
import { StorageArtifact } from "./storage";

export interface JobArtifact {
  job: app.Job;
  name: pulumi.Output<string>;
}

export interface JobInputs {
  rg: resources.ResourceGroup;
  env: app.ManagedEnvironment;
  registry: RegistryArtifact;
  storage: StorageArtifact;
  imageTag: string;
  cpu: string;
  memory: string;
  corpusPaths: string;
}

export function createIndexerJob(inputs: JobInputs): JobArtifact {
  const { rg, env, registry, storage, imageTag, cpu, memory, corpusPaths } = inputs;

  const imageRef = pulumi.interpolate`${registry.loginServer}/ministr:${imageTag}`;

  const job = new app.Job(named("indexer"), {
    resourceGroupName: rg.name,
    jobName: named("indexer"),
    location,
    environmentId: env.id,
    configuration: {
      triggerType: "Manual",
      replicaTimeout: 3600, // 1h hard cap (matches ACA Jobs default ceiling).
      replicaRetryLimit: 0,
      manualTriggerConfig: {
        replicaCompletionCount: 1,
        parallelism: 1,
      },
      registries: [
        {
          server: registry.loginServer,
          username: registry.adminUsername,
          passwordSecretRef: "registry-password",
        },
      ],
      secrets: [{ name: "registry-password", value: registry.adminPassword }],
    },
    template: {
      containers: [
        {
          name: "indexer",
          image: imageRef,
          resources: { cpu: Number(cpu), memory },
          // Indexer-mode entrypoint is selected by env var; the image
          // contains both serve and index entrypoints (PR2/PR5).
          env: [
            { name: "MINISTR_CLOUD_DATA_DIR", value: "/data" },
            { name: "MINISTR_CORPUS_PATHS", value: corpusPaths },
            { name: "ENTRYPOINT_MODE", value: "index" },
            { name: "RUST_LOG", value: "info,ministr=debug" },
          ],
          volumeMounts: [{ volumeName: "data", mountPath: "/data" }],
        },
      ],
      volumes: [
        {
          name: "data",
          storageType: "AzureFile",
          storageName: storage.storageName,
        },
      ],
    },
  });

  return { job, name: job.name };
}
