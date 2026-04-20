//! Shared bearer-token resolver for `/hooks/ingest` (Sub-plan 2) and `/mcp`
//! (this plan).
//!
//! Constant-time comparison via `subtle` to avoid timing oracles on token
//! length / content. Resolves token → session_id by querying
//! `sessions.api_token`.

use crate::{db::Db, error::Error};
use subtle::ConstantTimeEq;
use uuid::Uuid;

/// Resolve a presented bearer token to its owning session id.
///
/// Iterates over every active token row in constant time per row (no early
/// return on equality) so timing reveals at most "is there at least one
/// active token?" — not which one matched or how long the prefix overlap was.
pub fn resolve_token(db: &Db, presented: &str) -> Result<Uuid, Error> {
    if presented.is_empty() {
        return Err(Error::Unauthorized);
    }
    let candidates = db.sessions_with_tokens()?;

    let mut found: Option<Uuid> = None;
    let presented_bytes = presented.as_bytes();
    for (sid, token) in candidates.iter() {
        // Constant-time compare; do NOT short-circuit.
        let eq: bool = token.as_bytes().ct_eq(presented_bytes).into();
        if eq && found.is_none() {
            found = Some(*sid);
        }
    }
    found.ok_or(Error::Unauthorized)
}
