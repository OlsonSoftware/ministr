// Azure Container Registry (Basic SKU, ~$5/mo) with admin credentials
// enabled so the Container App can pull without managed-identity setup.
// Managed identity is a v2 hardening step.

import * as pulumi from "@pulumi/pulumi";
import * as cr from "@pulumi/azure-native/containerregistry";
import * as resources from "@pulumi/azure-native/resources";

import { location, named, registryName } from "./naming";

export interface RegistryArtifact {
  registry: cr.Registry;
  loginServer: pulumi.Output<string>;
  adminUsername: pulumi.Output<string>;
  adminPassword: pulumi.Output<string>;
}

export interface RegistryInputs {
  rg: resources.ResourceGroup;
}

export function createRegistry({ rg }: RegistryInputs): RegistryArtifact {
  const registry = new cr.Registry(named("acr"), {
    resourceGroupName: rg.name,
    registryName: registryName(),
    location,
    sku: { name: "Basic" },
    adminUserEnabled: true,
  });

  const credsOutput = pulumi
    .all([rg.name, registry.name])
    .apply(([rgName, regName]) =>
      cr.listRegistryCredentials({
        resourceGroupName: rgName,
        registryName: regName,
      }),
    );

  return {
    registry,
    loginServer: registry.loginServer,
    adminUsername: credsOutput.apply((c) => c.username ?? ""),
    adminPassword: credsOutput.apply((c) => c.passwords?.[0]?.value ?? ""),
  };
}
