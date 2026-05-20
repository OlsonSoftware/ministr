// PHASE5 chunk 1 — grant the serve pod's managed identity the
// `Container Apps Jobs Operator` built-in role, scoped to the indexer
// Job. This is the RBAC role PHASE3 chunk 6 declined to add (and the
// reason it routed through KEDA polling instead of a direct ARM call)
// — see the `feedback-no-rbac-deferral` memory and `PHASE5.md`.
//
// SRP: only this role-on-this-scope lives here. The blob-data role
// lives in `lib/role-assignment.ts`. Both files share the same
// deterministic-GUID pattern (see role-assignment.ts's preamble for
// the long-form rationale).
//
// # The role
//
// `Container Apps Jobs Operator` — built-in, GUID
// `b9a307c4-5aa3-4b52-ba60-2b17c136cd7b`. Grants read + start + stop
// on Container Apps Jobs. We scope it to the *specific* indexer Job,
// not the resource group or subscription, so the serve pod cannot
// start any other job in the deployment.
//
// # The principal
//
// The serve pod's SystemAssigned managed identity (`createApp` in
// `lib/app.ts` returns its `principalId`). The serve pod is the
// *caller* of ARM `POST /jobs/start`, so the role goes on its MI.
// The indexer Job's own MI stays scoped to blob-data; it does not
// need to start itself.
//
// # Failure mode without this role
//
// `POST /subscriptions/.../jobs/{name}/start` returns 403 Forbidden.
// The Rust trigger (`AcaJobStartTrigger`) surfaces the failure as
// `JobStartError::Arm { status: 403, ... }`, logs at warn, and the
// KEDA safety-net poll (5-min cadence per PHASE5's `lib/job.ts`
// change) picks the row up.

import * as crypto from "crypto";

import * as pulumi from "@pulumi/pulumi";
import * as authorization from "@pulumi/azure-native/authorization";
import * as app from "@pulumi/azure-native/app";

// Built-in role definition ID. Stable Azure GUID, sourced from
// learn.microsoft.com/azure/role-based-access-control/built-in-roles
// /containers#container-apps-jobs-operator.
const CONTAINER_APPS_JOBS_OPERATOR =
  "b9a307c4-5aa3-4b52-ba60-2b17c136cd7b";

/**
 * UUID-v5-style GUID derived from the joined inputs. Stable across
 * runs as long as the inputs are stable. Mirrors
 * `lib/role-assignment.ts::deterministicAssignmentGuid` exactly — see
 * that file's preamble for the rationale on why this matters for
 * Pulumi state hygiene.
 */
function deterministicAssignmentGuid(...parts: string[]): string {
  const hash = crypto.createHash("sha1").update(parts.join("|")).digest();
  const bytes = Array.from(hash.subarray(0, 16));
  bytes[6] = (bytes[6] & 0x0f) | 0x50;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = bytes
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20, 32)}`;
}

export interface JobsOperatorInputs {
  /** Resource name to use for the assignment (Pulumi logical name). */
  name: string;
  /** Indexer ACA Job — the scope the role is granted on. */
  indexerJob: app.Job;
  /** Principal id of the identity being granted the role. */
  principalId: pulumi.Input<string>;
}

/** Grant the principal `Container Apps Jobs Operator` on the indexer Job. */
export function grantJobsOperator(
  inputs: JobsOperatorInputs,
): authorization.RoleAssignment {
  const scope = inputs.indexerJob.id;
  const subscriptionId =
    authorization.getClientConfigOutput().subscriptionId;
  const roleDefinitionId = pulumi.interpolate`/subscriptions/${subscriptionId}/providers/Microsoft.Authorization/roleDefinitions/${CONTAINER_APPS_JOBS_OPERATOR}`;

  const roleAssignmentName = pulumi
    .all([scope, inputs.principalId, CONTAINER_APPS_JOBS_OPERATOR])
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
      // Same ignoreChanges + replaceOnChanges rationale as
      // `grantBlobDataContributor` — Azure's RBAC API returns
      // normalised values that diff against Pulumi's stored form, and
      // RBAC fields are immutable post-creation. See
      // `lib/role-assignment.ts` for the full long-form justification.
      ignoreChanges: ["scope", "principalType", "roleAssignmentName"],
      replaceOnChanges: ["principalId"],
    },
  );
}
