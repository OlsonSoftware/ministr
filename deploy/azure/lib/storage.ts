// Storage Account + File Share + ManagedEnvironmentsStorage mount.
//
// One Azure Files share holds:
//   /data/oauth.db        — OAuth state (PR1.2)
//   /data/jobs.db         — Indexer job queue (PR1.3)
//   /data/corpora/...     — SQLite + HNSW per indexed corpus (PR2)
//
// Both the query App and the Indexer Job mount this same share at /data,
// so the App reads what the Job writes (and vice versa for OAuth/jobs).

import * as pulumi from "@pulumi/pulumi";
import * as resources from "@pulumi/azure-native/resources";
import * as storage from "@pulumi/azure-native/storage";
import * as app from "@pulumi/azure-native/app";

import { location, named, storageAccountName } from "./naming";

export interface StorageArtifact {
  account: storage.StorageAccount;
  share: storage.FileShare;
  accountName: pulumi.Output<string>;
  accountKey: pulumi.Output<string>;
  shareName: pulumi.Output<string>;
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
    accountName: account.name,
    accountKey,
    shareName: share.name,
    envStorage,
    storageName: envStorage.name,
  };
}
