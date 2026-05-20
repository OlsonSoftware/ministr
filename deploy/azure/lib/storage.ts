// Storage Account + Blob Container + (legacy) File Share.
//
// Architecture (post-Option B):
//   - **Blob container** `ministr-corpora` is the source of truth for
//     HNSW bundles. `ministr_cloud::CorpusBlobStore` reads/writes
//     versioned `.ministr-index` bundles + a small manifest pointer.
//   - **Pod-ephemeral `/data`** holds the working SQLite + HNSW that
//     the query app reads (container filesystem, supports WAL). At
//     boot the app calls `download_corpus` for every blob bundle.
//     On shutdown / after each ingest the app calls `upload_corpus`.
//   - **Postgres** holds OAuth state, tenancy metadata, usage events,
//     audit log (see lib/postgres.ts).
//
// The Azure Files share is kept for the v1 indexer-job mount but the
// query app no longer mounts it (SMB can't host SQLite WAL).

import * as pulumi from "@pulumi/pulumi";
import * as resources from "@pulumi/azure-native/resources";
import * as storage from "@pulumi/azure-native/storage";
import * as app from "@pulumi/azure-native/app";

import { location, named, storageAccountName } from "./naming";

export interface StorageArtifact {
  account: storage.StorageAccount;
  share: storage.FileShare;
  blobContainer: storage.BlobContainer;
  accountName: pulumi.Output<string>;
  accountKey: pulumi.Output<string>;
  shareName: pulumi.Output<string>;
  blobContainerName: pulumi.Output<string>;
  envStorage: app.ManagedEnvironmentsStorage;
  /** ACA `storageName` to reference from container-app volumes. */
  storageName: pulumi.Output<string>;
}

export interface StorageInputs {
  rg: resources.ResourceGroup;
  env: app.ManagedEnvironment;
}

export function createStorage({ rg, env }: StorageInputs): StorageArtifact {
  const account = new storage.StorageAccount(named("sa"), {
    resourceGroupName: rg.name,
    accountName: storageAccountName("data"),
    location,
    kind: "StorageV2",
    sku: { name: "Standard_LRS" },
    accessTier: "Hot",
    allowSharedKeyAccess: true,
    minimumTlsVersion: "TLS1_2",
  });

  const keys = pulumi
    .all([rg.name, account.name])
    .apply(([rgName, acctName]) =>
      storage.listStorageAccountKeys({
        resourceGroupName: rgName,
        accountName: acctName,
      }),
    );
  const accountKey = keys.apply((k) => k.keys?.[0]?.value ?? "");

  const fileService = new storage.FileServiceProperties(named("fileservice"), {
    resourceGroupName: rg.name,
    accountName: account.name,
    fileServicesName: "default",
  });

  const share = new storage.FileShare(
    named("share"),
    {
      resourceGroupName: rg.name,
      accountName: account.name,
      shareName: "ministr-data",
      shareQuota: 10,
      accessTier: "Hot",
    },
    { dependsOn: [fileService] },
  );

  // Blob container for HNSW bundles. `CorpusBlobStore` reads/writes
  // `corpora/<id>/manifest.json` (pointer) + `corpora/<id>/<version>.ministr-index`
  // (versioned bundle). Container name must match
  // MINISTR_BLOB_AZURE_CONTAINER set in lib/app.ts.
  const blobContainer = new storage.BlobContainer(named("corpora"), {
    resourceGroupName: rg.name,
    accountName: account.name,
    containerName: "ministr-corpora",
    publicAccess: storage.PublicAccess.None,
  });

  // Container Apps environment mount declaration — referenced by name from
  // each container-app revision's `template.volumes`.
  const envStorage = new app.ManagedEnvironmentsStorage(named("envstorage"), {
    resourceGroupName: rg.name,
    environmentName: env.name,
    storageName: "ministr-data",
    properties: {
      azureFile: {
        accountName: account.name,
        accountKey,
        shareName: share.name,
        accessMode: "ReadWrite",
      },
    },
  });

  return {
    account,
    share,
    blobContainer,
    accountName: account.name,
    accountKey,
    shareName: share.name,
    blobContainerName: blobContainer.name,
    envStorage,
    storageName: envStorage.name,
  };
}
