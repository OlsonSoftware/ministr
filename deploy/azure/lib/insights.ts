// Application Insights component — feeds the Tauri cost/latency badges.
// Linked to the same Log Analytics workspace as the ACA env so logs and
// metrics live together.

import * as pulumi from "@pulumi/pulumi";
import * as insights from "@pulumi/azure-native/applicationinsights";
import * as resources from "@pulumi/azure-native/resources";
import * as op from "@pulumi/azure-native/operationalinsights";

import { location, named } from "./naming";

export interface InsightsArtifact {
  component: insights.Component;
  connectionString: pulumi.Output<string>;
}

export interface InsightsInputs {
  rg: resources.ResourceGroup;
  workspace: op.Workspace;
}

export function createInsights({ rg, workspace }: InsightsInputs): InsightsArtifact {
  const component = new insights.Component(named("ai"), {
    resourceGroupName: rg.name,
    resourceName: named("ai"),
    location,
    kind: "web",
    applicationType: "web",
    workspaceResourceId: workspace.id,
  });

  return {
    component,
    connectionString: component.connectionString,
  };
}
