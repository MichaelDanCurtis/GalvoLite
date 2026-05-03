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
