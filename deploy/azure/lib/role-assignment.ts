// RBAC bindings for the cloud's managed identities.
//
// The containers' `azure_identity::ManagedIdentityCredential` chain
// (used by `ministr_cloud::CorpusBlobStore`) authenticates as each
// ContainerApp/Job's SystemAssigned identity. To actually read/write
// blobs each MI needs `Storage Blob Data Contributor` on the storage
// account.
//
// SRP: only role bindings live here. Provisioning the identity itself
// is in lib/app.ts (query app) / lib/job.ts (indexer worker).
//
// # Deterministic role-assignment GUID
//
// Azure RBAC role assignments are uniquely identified by a GUID — the
// `name` field of the `Microsoft.Authorization/roleAssignments` resource.
// If you don't pass `roleAssignmentName`, Azure auto-generates one;
// Pulumi then tracks that random GUID in state. That seems harmless
// but it has a sharp edge: when the principal rotates (e.g. a Container
// App's SystemAssigned MI gets a new principalId across certain replace
// conditions), Pulumi tries to update — Azure rejects because principal
// is immutable on a role assignment — Pulumi falls back to replace —
// but with an auto-generated name the new resource has a brand-new
// GUID, and the old GUID can be left orphaned on the scope if the
// delete-after-create half fails. Repeated apply/replace cycles
// accumulate orphan grants pointing at long-dead phantom principals.
//
// We derive the GUID from `(scope, principalId, roleDefinitionId)` as
// a UUID-v5 hash. This gives us:
//
//   - Idempotency: same (principal, scope, role) → same GUID. Re-applying
//     never creates a duplicate.
//   - Clean replace on principal rotation: changing principalId changes
//     the GUID, so Pulumi sees a genuine replace (delete-old + create-new)
//     instead of a wedged in-place update.
//   - Self-converging: no compounding orphan accumulation.
//
// Pre-existing orphans (from before this change) are not cleaned up
// here — Pulumi can only delete resources it owns. They're harmless
// no-ops (granting blob access to principals that no longer exist),
// removable via `az role assignment delete` if you want a clean
// `az role assignment list`.

import * as crypto from "crypto";

import * as pulumi from "@pulumi/pulumi";
import * as authorization from "@pulumi/azure-native/authorization";
import * as storage from "@pulumi/azure-native/storage";

// Built-in role definition ID for Storage Blob Data Contributor.
// Stable Azure GUID; see
// learn.microsoft.com/azure/role-based-access-control/built-in-roles
const STORAGE_BLOB_DATA_CONTRIBUTOR =
  "ba92f5b4-2d11-453d-a403-e96b0029c9fe";

// Built-in role definition ID for Cognitive Services User. Grants
// the principal "Read Cognitive Services data and write completions".
// Sufficient for the embedder's POST /embeddings + MI bearer auth.
// learn.microsoft.com/azure/role-based-access-control/built-in-roles/ai-machine-learning#cognitive-services-user
const COGNITIVE_SERVICES_USER =
  "a97b65f3-24c7-4388-baec-2e87135dc908";

/**
 * UUID-v5-style GUID derived from the joined inputs. Stable across
 * runs as long as the inputs are stable. Used to give each role
 * assignment a deterministic name so Pulumi never thinks two
 * `(principal, scope, role)` triples should share the same Azure
 * resource.
 */
function deterministicAssignmentGuid(...parts: string[]): string {
  const hash = crypto.createHash("sha1").update(parts.join("|")).digest();
  const bytes = Array.from(hash.subarray(0, 16));
  // RFC 4122 v5: top nibble of byte 6 = version (5).
  bytes[6] = (bytes[6] & 0x0f) | 0x50;
  // RFC 4122: top two bits of byte 8 = variant (10).
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = bytes
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20, 32)}`;
}

export interface BlobDataContributorInputs {
  /** Resource name to use for the assignment (Pulumi logical name). */
  name: string;
  /** Storage account the role is scoped to. */
  storageAccount: storage.StorageAccount;
  /** Principal id of the identity being granted the role. */
  principalId: pulumi.Input<string>;
}

/** Grant the principal Storage Blob Data Contributor on the account. */
export function grantBlobDataContributor(
  inputs: BlobDataContributorInputs,
): authorization.RoleAssignment {
  const scope = inputs.storageAccount.id;
  const subscriptionId =
    authorization.getClientConfigOutput().subscriptionId;
  const roleDefinitionId = pulumi.interpolate`/subscriptions/${subscriptionId}/providers/Microsoft.Authorization/roleDefinitions/${STORAGE_BLOB_DATA_CONTRIBUTOR}`;

  // Compose the deterministic name from the three identity-bearing
  // inputs. `pulumi.all` waits for each Output to materialise; the
  // returned Output is the v5 GUID. Pulumi feeds this back into the
  // RoleAssignment's `roleAssignmentName` so the resulting Azure ID is
  // `<scope>/providers/Microsoft.Authorization/roleAssignments/<guid>`.
  const roleAssignmentName = pulumi
    .all([scope, inputs.principalId, STORAGE_BLOB_DATA_CONTRIBUTOR])
    .apply(([s, p, r]) => deterministicAssignmentGuid(s, p, r));

  return new authorization.RoleAssignment(
    inputs.name,
    {
      roleAssignmentName,
      principalId: inputs.principalId,
      principalType: authorization.PrincipalType.ServicePrincipal,
      roleDefinitionId,
      scope,
    },
    {
      // # ignoreChanges
      //
      // - `scope`: Azure normalises this on read (returns it without
      //   the leading `/`) while `StorageAccount.id` has one. Pulumi
      //   otherwise sees a phantom diff every apply and tries to
      //   replace, which (a) collides with the deterministic GUID
      //   since the replacement target name is identical, and (b)
      //   re-introduces orphan accumulation.
      // - `principalType`: same normalisation hazard (enum vs. string
      //   casing on read).
      // - `roleAssignmentName`: imported resources from earlier deploys
      //   carry Azure-auto-generated GUIDs in state (version 4/8 random),
      //   but our `deterministicAssignmentGuid` produces UUID-v5. Pulumi
      //   would otherwise diff on every apply and try to replace, which
      //   re-wedges (the deterministic target name conflicts with the
      //   live resource at the same (principal, scope, role) triple).
      //   The deterministic name is still used at CREATE time for fresh
      //   resources — the ignore only affects subsequent applies, so
      //   existing imports settle without churn.
      //
      // All three fields are immutable post-creation on Azure RBAC, so
      // ignoring future diffs is semantically a no-op: there's nothing
      // legitimate to ever update there.
      ignoreChanges: ["scope", "principalType", "roleAssignmentName"],
      // # replaceOnChanges
      //
      // Pulumi's default would diff `principalId` and try to update —
      // which Azure rejects (immutable), and Pulumi falls back to
      // replace. With deterministic naming the replace itself wedges
      // (409 on the new GUID). Marking `principalId` as
      // replace-triggering makes Pulumi treat principal rotation as a
      // genuine delete+create up front — the only correct behaviour.
      replaceOnChanges: ["principalId"],
    },
  );
}

export interface CognitiveServicesUserInputs {
  /** Resource name to use for the assignment (Pulumi logical name). */
  name: string;
  /** Cognitive Services / Azure OpenAI account id the role is scoped to. */
  accountId: pulumi.Input<string>;
  /** Principal id of the identity being granted the role. */
  principalId: pulumi.Input<string>;
}

/** Grant the principal `Cognitive Services User` on the OpenAI account.
 *
 * PHASE6 chunk 4a — the serve pod's MI mints a Bearer for
 * `https://cognitiveservices.azure.com` and the embedder's
 * `POST /embeddings` call needs this role on the OpenAI account.
 * Without it, the call returns 403. Same deterministic-GUID pattern
 * as `grantBlobDataContributor` — see that function's preamble for
 * the rationale. */
export function grantCognitiveServicesUser(
  inputs: CognitiveServicesUserInputs,
): authorization.RoleAssignment {
  const subscriptionId =
    authorization.getClientConfigOutput().subscriptionId;
  const roleDefinitionId = pulumi.interpolate`/subscriptions/${subscriptionId}/providers/Microsoft.Authorization/roleDefinitions/${COGNITIVE_SERVICES_USER}`;

  const roleAssignmentName = pulumi
    .all([inputs.accountId, inputs.principalId, COGNITIVE_SERVICES_USER])
    .apply(([s, p, r]) => deterministicAssignmentGuid(s, p, r));

  return new authorization.RoleAssignment(
    inputs.name,
    {
      roleAssignmentName,
      principalId: inputs.principalId,
      principalType: authorization.PrincipalType.ServicePrincipal,
      roleDefinitionId,
      scope: inputs.accountId,
    },
    {
      ignoreChanges: ["scope", "principalType", "roleAssignmentName"],
      replaceOnChanges: ["principalId"],
    },
  );
}
