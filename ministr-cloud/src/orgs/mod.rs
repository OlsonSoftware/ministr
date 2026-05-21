//! Multi-tenant orgs (F3.1a).
//!
//! The `orgs` + `org_members` tables shipped with the F1.2 initial
//! migration (`migrations/0001_initial.sql`). This module is the first
//! consumer of those tables: it adds the `orgs` repo helpers and the
//! `/api/v1/orgs/...` HTTP surface that the Tauri panel and the future
//! `/orgs/new` web flow drive.
//!
//! # Roadmap split
//!
//! F3.1 was originally drafted as one chunk but was too large to verify
//! in a single pass. F3.1a (this module) ships the CRUD + listing
//! surface; F3.1b adds magic-link email invites on top; F3.1c wires
//! Stripe seat-quantity sync. The shape here is intentionally limited
//! to "owner creates an org and sees its members" so each later sub-
//! chunk can extend without retrofitting.
//!
//! # Open-core posture
//!
//! Lives in `ministr-cloud` (closed) because orgs are a multi-tenant-
//! cloud concept — self-hosted single-user serve never mounts these
//! routes (`cmd_serve_http` only merges `orgs_routes` when
//! `cloud_pool.is_some()`).

pub mod corpus_acl;
pub mod invites;
pub mod repo;
pub mod routes;
pub mod seats;

pub use corpus_acl::{
    AclEntry, acl_grants_access, corpus_owner_tenant, list_acl, revoke_org_share, share_with_org,
};
pub use invites::{
    CreatedInvite, ConsumeOutcome, DEFAULT_INVITE_TTL, InviteRow, consume_invite, create_invite,
};
pub use repo::{
    DEFAULT_ORG_PLAN, MemberRow, OrgError, OrgRow, OrgWithRole, create_org, list_org_members,
    list_orgs_for_user, member_role, set_org_stripe_customer_id, user_email,
};
pub use routes::{OrgsState, orgs_routes};
pub use seats::{SeatsSyncError, SeatsSyncOutcome, count_org_members, sync_org_seats};
