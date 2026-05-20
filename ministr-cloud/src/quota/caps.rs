//! `Plan` → tier-cap mapping. Pure data; no I/O.
//!
//! `None` means unlimited. Caps are read once per request out of a
//! constant function, so adding a new cap field is a single-line
//! change in two places (this module + a new [`crate::quota::rule`]
//! impl).
//!
//! Values mirror ROADMAP §3 verbatim. Treat that section as the
//! source of truth; if the numbers change there, change them here in
//! the same commit.

use ministr_mcp::auth::Plan;

/// Per-tier caps the quota middleware enforces. `None` means
/// "unlimited" (Enterprise sits there for the corpus cap; Pro and Team
/// have explicit ceilings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanCaps {
    /// Maximum hosted corpora on the cloud. Pro = 10, Team = 50,
    /// Enterprise = unlimited per §3.
    pub corpora: Option<u64>,
}

/// Resolve the caps a tenant on `plan` operates under. `const fn` so
/// the lookup compiles to a jump table.
#[must_use]
pub const fn caps_for_plan(plan: Plan) -> PlanCaps {
    match plan {
        Plan::Pro => PlanCaps {
            corpora: Some(10),
        },
        Plan::Team => PlanCaps {
            corpora: Some(50),
        },
        Plan::Enterprise => PlanCaps { corpora: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pro_cap_matches_roadmap_section_3() {
        assert_eq!(caps_for_plan(Plan::Pro).corpora, Some(10));
    }

    #[test]
    fn team_cap_matches_roadmap_section_3() {
        assert_eq!(caps_for_plan(Plan::Team).corpora, Some(50));
    }

    #[test]
    fn enterprise_is_unlimited() {
        assert!(caps_for_plan(Plan::Enterprise).corpora.is_none());
    }
}
