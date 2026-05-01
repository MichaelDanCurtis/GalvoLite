//! Tool definitions, organized by tier and topic.
//!
//! Tasks 6-10 fill in each module. For now `all()` returns an empty Vec so the
//! registry compiles; tools/list will be empty until later tasks land.

use crate::mcp::registry::ToolDef;

pub mod spawn;
pub mod tier1_introspection;
pub mod tier1_role_handoff;
pub mod tier1_status;
pub mod tier2_3_cross;
pub mod tier4_stubs;

pub fn all() -> Vec<ToolDef> {
    let mut v = Vec::new();
    v.extend(tier1_status::tools());
    v.extend(tier1_role_handoff::tools());
    v.extend(tier1_introspection::tools());
    v.extend(tier2_3_cross::tools());
    v.extend(tier4_stubs::tools());
    v.extend(spawn::tools());
    v
}
