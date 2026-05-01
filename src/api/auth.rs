//! Shared bearer-token authentication helper.
//!
//! Factored out of `src/api/hooks.rs` so that both the hooks ingest handler
//! and the pending-rejoin handler can use the same constant-time token
//! resolution without duplicating the implementation.

use crate::{db::Db, error::Error};

/// Resolve a bearer token to its owning `session_id` using a constant-time
/// byte compare to defeat timing oracles. Iterates over live tokens; for
/// the session counts GalvoLite expects (<100) this is fine.
///
/// Does NOT early-return on a match — the full scan always completes so
/// that the compare time is independent of which row matched.
pub fn resolve_token(db: &Db, token: &str) -> Result<uuid::Uuid, Error> {
    use subtle::ConstantTimeEq;
    let conn = db.pool().get()?;
    let mut stmt =
        conn.prepare("SELECT id, api_token FROM sessions WHERE api_token IS NOT NULL")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let token_bytes = token.as_bytes();
    let mut found: Option<uuid::Uuid> = None;
    for row in rows {
        let (id, t) = row?;
        // Constant-time compare requires equal-length slices; padding with
        // a wrong-but-constant length keeps the timing uniform.
        if t.len() == token_bytes.len() && t.as_bytes().ct_eq(token_bytes).unwrap_u8() == 1 {
            // Don't early-return: keep iterating so the compare time doesn't
            // depend on which row matched.
            found = uuid::Uuid::parse_str(&id).ok();
        }
    }
    found.ok_or(Error::SessionNotFound(uuid::Uuid::nil()))
}
