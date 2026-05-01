//! Rejoin marker detection. The harness emits one of these to wake a parent
//! whose children have all reached terminal status. The galvolite-hook
//! binary recognises the marker and replaces it with the authoritative
//! markdown payload fetched from /api/sessions/:id/pending-rejoin.

use uuid::Uuid;

/// Returns the parent session id if the trimmed prompt is exactly the marker.
pub fn detect_rejoin_marker(prompt: &str) -> Option<Uuid> {
    let trimmed = prompt.trim();
    let prefix = "[GALVOLITE:REJOIN ";
    let suffix = "]";
    let inner = trimmed.strip_prefix(prefix)?.strip_suffix(suffix)?;
    Uuid::parse_str(inner).ok()
}
