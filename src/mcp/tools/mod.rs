//! Tool definitions, organized by tier and topic.
//!
//! Tasks 6-10 fill in each module. For now `all()` returns an empty Vec so the
//! registry compiles; tools/list will be empty until later tasks land.

use crate::mcp::registry::ToolDef;

pub mod tier1_role_handoff;
pub mod tier1_status;

pub fn all() -> Vec<ToolDef> {
    let mut v = Vec::new();
    v.extend(tier1_status::tools());
    v.extend(tier1_role_handoff::tools());
    // Task 8 — Tier 1 introspection:   v.extend(tier1_introspection::tools());
    // Task 9 — Tier 2/3 cross-session: v.extend(tier2_3_cross::tools());
    // Task 10 — Tier 4 reserved stubs: v.extend(tier4_stubs::tools());
    v
}
