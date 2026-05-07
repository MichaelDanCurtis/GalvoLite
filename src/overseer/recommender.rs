//! Prompt packing + response parsing for the Overseer Kimi calls.

use serde::Deserialize;

use super::observer::ContextSnapshot;

const PROMPT_BUDGET: usize = 8_000;

const SYSTEM: &str = "\
You are the GalvoLite Overseer. You watch a Claude-driven coding project from the side, \
using the bus events, plan, directive, postmortems, and decisions provided. You do NOT \
write code. Your only output is a JSON object with key `recommendations`, each item \
having {kind, severity (info|warn|urgent), title, body, rationale, evidence_refs}. Be \
concise: one or two sentences per body. Skip noise — emit at most 5 recommendations.";

pub fn pack_prompt(snap: &ContextSnapshot) -> (String, String) {
    let mut user = String::new();
    user.push_str("# Recent events\n\n");
    for e in snap.events.iter().rev().take(40) {
        user.push_str(&format!("- {} ({}, {}) {}\n",
            e.kind, e.source, &e.ts, summarize_payload(&e.payload)));
    }
    if let Some(p) = &snap.plan_md {
        user.push_str("\n# plan.md\n\n");
        push_capped(&mut user, p, 1500);
    }
    if let Some(d) = &snap.directive_md {
        user.push_str("\n# directive.md\n\n");
        push_capped(&mut user, d, 1500);
    }
    if !snap.recent_postmortems.is_empty() {
        user.push_str("\n# Recent postmortems\n\n");
        for pm in snap.recent_postmortems.iter().take(3) {
            user.push_str(&format!("- part {} ({}): gotchas={}\n",
                pm.plan_part, pm.outcome, first_line(&pm.gotchas)));
        }
    }
    if !snap.recent_decisions.is_empty() {
        user.push_str("\n# Recent decisions\n\n");
        for d in snap.recent_decisions.iter().take(5) {
            user.push_str(&format!("- Q: {}\n  A: {}\n", first_line(&d.question), first_line(&d.answer)));
        }
    }
    if !snap.tail_per_session.is_empty() {
        user.push_str("\n# Recent PTY tails\n\n");
        // Cap each tail to keep total prompt under PROMPT_BUDGET.
        for (sid, tail) in snap.tail_per_session.iter().take(5) {
            user.push_str(&format!("## Session {sid}\n\n```\n"));
            push_capped(&mut user, tail.trim(), 800);
            user.push_str("\n```\n\n");
        }
    }
    if user.len() > PROMPT_BUDGET { user.truncate(PROMPT_BUDGET); }
    (SYSTEM.to_string(), user)
}

fn push_capped(out: &mut String, s: &str, cap: usize) {
    if s.len() <= cap { out.push_str(s); } else { out.push_str(&s[..cap]); out.push_str("\n…(truncated)\n"); }
}

fn first_line(s: &str) -> &str { s.lines().next().unwrap_or("") }

fn summarize_payload(v: &serde_json::Value) -> String {
    if let Some(o) = v.as_object() {
        let kvs: Vec<String> = o.iter().take(3)
            .map(|(k, v)| format!("{k}={}", v.to_string().chars().take(40).collect::<String>()))
            .collect();
        kvs.join(" ")
    } else { String::new() }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParsedRec {
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub rationale: String,
    #[serde(default)]
    pub evidence_refs: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Wrapped { recommendations: Vec<ParsedRec> }

/// Parse Kimi's response. Tolerates fenced code blocks and surrounding chatter.
/// Returns Ok([]) on malformed input — never propagates parse error.
pub fn parse_kimi_response(s: &str) -> Result<Vec<ParsedRec>, std::convert::Infallible> {
    let candidate = extract_json_block(s).unwrap_or(s);
    Ok(serde_json::from_str::<Wrapped>(candidate)
        .map(|w| w.recommendations)
        .unwrap_or_default())
}

fn extract_json_block(s: &str) -> Option<&str> {
    if let Some(start) = s.find("```json") {
        let after = &s[start + 7..];
        let end = after.find("```")?;
        return Some(after[..end].trim());
    }
    if let (Some(a), Some(b)) = (s.find('{'), s.rfind('}')) {
        if a < b { return Some(&s[a..=b]); }
    }
    None
}


use crate::api::events::{DomainEvent, EventBroadcaster};
use crate::db::Db;
use crate::recommendations::{insert, NewRec};
use std::future::Future;
use uuid::Uuid;

pub struct OverseerState {
    pub daily_budget: u64,
    pub tokens_used: u64,
    pub consecutive_failures: u32,
    pub degraded: bool,
}

impl OverseerState {
    pub fn new(daily_budget: u64) -> Self {
        Self { daily_budget, tokens_used: 0, consecutive_failures: 0, degraded: false }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("budget exhausted ({used}/{budget})")] Budget { used: u64, budget: u64 },
    #[error("kimi: {0}")] Kimi(#[from] crate::overseer::kimi::KimiError),
    #[error("db: {0}")] Db(#[from] crate::error::Error),
}

/// Run one review cycle. `kimi_call` is a closure injected for testability —
/// in production it wraps `KimiClient::chat`.
pub async fn review<F, Fut>(
    db: &Db,
    bus: &EventBroadcaster,
    project_id: Uuid,
    snap: &ContextSnapshot,
    state: &mut OverseerState,
    kimi_call: F,
) -> Result<usize, ReviewError>
where
    F: FnOnce(String, String) -> Fut,
    Fut: Future<Output = Result<(String, u64), crate::overseer::kimi::KimiError>>,
{
    if state.tokens_used >= state.daily_budget {
        return Err(ReviewError::Budget { used: state.tokens_used, budget: state.daily_budget });
    }
    let (sys, user) = pack_prompt(snap);
    match kimi_call(sys, user).await {
        Ok((text, tokens)) => {
            state.tokens_used = state.tokens_used.saturating_add(tokens);
            if state.degraded {
                bus.publish(DomainEvent::OverseerRecovered { project_id });
                state.degraded = false;
            }
            state.consecutive_failures = 0;
            let recs = parse_kimi_response(&text).unwrap_or_default();
            let mut inserted = 0;
            for r in recs {
                let nr = NewRec {
                    project_id,
                    kind: r.kind, severity: r.severity, title: r.title,
                    body: r.body, rationale: r.rationale,
                    evidence_json: serde_json::Value::Array(r.evidence_refs),
                };
                let sev = nr.severity.clone();
                if let Some(id) = insert(db, nr)? {
                    bus.publish(DomainEvent::RecommendationCreated {
                        rec_id: id, project_id, severity: sev,
                    });
                    inserted += 1;
                }
            }
            Ok(inserted)
        }
        Err(e) => {
            state.consecutive_failures += 1;
            if state.consecutive_failures >= 5 && !state.degraded {
                bus.publish(DomainEvent::OverseerDegraded {
                    project_id, reason: format!("{e}"),
                });
                state.degraded = true;
            }
            Err(ReviewError::Kimi(e))
        }
    }
}
