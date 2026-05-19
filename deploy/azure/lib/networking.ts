// Foundation resources: Resource Group, Log Analytics workspace, and the
// Container Apps Managed Environment. Every other resource lives inside
// the RG and references the env.

import * as pulumi from "@pulumi/pulumi";
import * as resources from "@pulumi/azure-native/resources";
import * as op from "@pulumi/azure-native/operationalinsights";
import * as app from "@pulumi/azure-native/app";

import { location, named } from "./naming";

export interface Networking {
  rg: resources.ResourceGroup;
  workspace: op.Workspace;
  workspaceCustomerId: pulumi.Output<string>;
  workspaceSharedKey: pulumi.Output<string>;
  env: app.ManagedEnvironment;
}

export function createNetworking(): Networking {
  const rg = new resources.ResourceGroup(named("rg"), {
    resourceGroupName: named("rg-prod"),
    location,
  });

  const workspace = new op.Workspace(named("logs"), {
    resourceGroupName: rg.name,
    workspaceName: named("logs"),
    location,
    sku: { name: "PerGB2018" },
    retentionInDays: 30,
  });

  const sharedKeys = pulumi
    .all([rg.name, workspace.name])
    .apply(([rgName, wsName]) =>
      op.getSharedKeys({ resourceGroupName: rgName, workspaceName: wsName }),
    );

  const env = new app.ManagedEnvironment(named("env"), {
    resourceGroupName: rg.name,
    environmentName: named("env"),
    location,
    appLogsConfiguration: {
      destination: "log-analytics",
      logAnalyticsConfiguration: {
        customerId: workspace.customerId,
        sharedKey: sharedKeys.apply((k) => k.primarySharedKey ?? ""),
      },
    },
    workloadProfiles: [
      { name: "Consumption", workloadProfileType: "Consumption" },
    ],
  });

  return {
    rg,
    workspace,
    workspaceCustomerId: workspace.customerId,
    workspaceSharedKey: sharedKeys.apply((k) => k.primarySharedKey ?? ""),
    env,
  };
}
