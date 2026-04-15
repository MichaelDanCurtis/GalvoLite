//! Signal protocol parsing.
//!
//! Signals are single-line markers Claude emits. See design spec §8.3 / Appendix A.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    StatusWorking,
    StatusComplete,
    AttentionNeeded { reason: String },
    AttentionContext,
    HandoffReady,
    HandoffBlocked { reason: String },
    PlanReady,
    SeedNew { id: String },
}

impl Signal {
    /// Stable string used for DB storage (kind column).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::StatusWorking => "STATUS:WORKING",
            Self::StatusComplete => "STATUS:COMPLETE",
            Self::AttentionNeeded { .. } => "ATTENTION:NEEDED",
            Self::AttentionContext => "ATTENTION:CONTEXT",
            Self::HandoffReady => "HANDOFF:READY",
            Self::HandoffBlocked { .. } => "HANDOFF:BLOCKED",
            Self::PlanReady => "PLAN:READY",
            Self::SeedNew { .. } => "SEED:NEW",
        }
    }

    pub fn payload(&self) -> Option<String> {
        match self {
            Self::AttentionNeeded { reason } | Self::HandoffBlocked { reason } => {
                Some(reason.clone())
            }
            Self::SeedNew { id } => Some(id.clone()),
            _ => None,
        }
    }
}

/// Parse a single line. Returns `Some(Signal)` iff the line, after trimming,
/// is exactly a well-formed signal. Any extra non-whitespace content → None.
pub fn parse_signal(line: &str) -> Option<Signal> {
    let line = line.trim();
    if !line.starts_with('[') || !line.ends_with(']') {
        return None;
    }
    let inner = &line[1..line.len() - 1];

    // Fast path: parameterless signals.
    match inner {
        "STATUS:WORKING" => return Some(Signal::StatusWorking),
        "STATUS:COMPLETE" => return Some(Signal::StatusComplete),
        "ATTENTION:CONTEXT" => return Some(Signal::AttentionContext),
        "HANDOFF:READY" => return Some(Signal::HandoffReady),
        "PLAN:READY" => return Some(Signal::PlanReady),
        _ => {}
    }

    // Parameterized signals: "<KIND> <key>=\"<value>\""
    let (kind, rest) = inner.split_once(' ')?;
    let rest = rest.trim();

    match kind {
        "ATTENTION:NEEDED" => {
            let reason = extract_quoted(rest, "reason")?;
            Some(Signal::AttentionNeeded { reason })
        }
        "HANDOFF:BLOCKED" => {
            let reason = extract_quoted(rest, "reason")?;
            Some(Signal::HandoffBlocked { reason })
        }
        "SEED:NEW" => {
            let id = extract_quoted(rest, "id")?;
            Some(Signal::SeedNew { id })
        }
        _ => None,
    }
}

/// Extract the value of `<key>="<value>"` from `s`. Supports backslash-escaped quotes.
fn extract_quoted(s: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=\"");
    let rest = s.strip_prefix(&prefix)?;
    let mut out = String::with_capacity(rest.len());
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let next = chars.next()?;
                out.push(next);
            }
            '"' => {
                // Expect end-of-string (maybe trailing space).
                let tail: String = chars.collect();
                if tail.trim().is_empty() {
                    return Some(out);
                }
                return None;
            }
            other => out.push(other),
        }
    }
    None
}
