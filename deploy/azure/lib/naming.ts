// Shared naming helpers. Centralised so resource names stay predictable
// and we can prepend the project prefix exactly once per resource.
//
// SRP: this file does naming and nothing else. Other libs import the
// helpers; never duplicate the prefix logic.

import * as pulumi from "@pulumi/pulumi";

const cfg = new pulumi.Config();

/** Project-wide prefix, e.g. "ministr". */
export const projectName = cfg.get("projectName") ?? "ministr";

/** Default Azure region for every resource. */
export const location = cfg.get("location") ?? "eastus";

/**
 * Compose a resource name as `<projectName>-<role>`.
 * Use for resource groups, container apps, etc. that allow hyphens.
 */
export const named = (role: string) => `${projectName}-${role}`;

/**
 * Compose a storage-account name: alphanumeric only, max 24 chars, lower-case.
 * Storage accounts have the strictest name rules in Azure.
 */
export const storageAccountName = (suffix: string) =>
  `${projectName}${suffix}`.toLowerCase().replace(/[^a-z0-9]/g, "").slice(0, 24);

/**
 * Compose a container-registry name: alphanumeric only, lower-case.
 */
export const registryName = () =>
  `${projectName}acr`.toLowerCase().replace(/[^a-z0-9]/g, "").slice(0, 50);
