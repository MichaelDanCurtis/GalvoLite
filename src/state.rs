//! State funnel for the hooks+MCP coordination layer.
//!
//! See `docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md`
//! §8 for the design. Every session-state transition in GalvoLite goes through
//! [`apply_event`] (added in Task 5); this module owns the canonical
//! [`SessionState`] enum and the event input/outcome types that funnel
//! describes.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Session lifecycle states from spec §8.1. The DB stores the snake_case
/// rendering returned by [`SessionState::fmt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Row inserted, child PTY not yet observed alive.
    Starting,
    /// PTY alive, no work in flight (default between turns).
    Idle,
    /// Model is mid-turn.
    Working,
    /// Model paused on an operator question (Finding 3 fix).
    AwaitingInput,
    /// Model asked for operator attention (`request_attention` /
    /// `status_report(blocked)`).
    NeedsAttention,
    /// PTY died unexpectedly, or row is left over from a prior schema.
    Error,
    /// Session ended cleanly (`SessionEnd` hook or `request_end` tool).
    Ended,
}

impl SessionState {
    /// `Error` and `Ended` are terminal — no transition out.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Error | Self::Ended)
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Starting       => "starting",
            Self::Idle           => "idle",
            Self::Working        => "working",
            Self::AwaitingInput  => "awaiting_input",
            Self::NeedsAttention => "needs_attention",
            Self::Error          => "error",
            Self::Ended          => "ended",
        })
    }
}

impl FromStr for SessionState {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "starting"        => Self::Starting,
            "idle"            => Self::Idle,
            "working"         => Self::Working,
            "awaiting_input"  => Self::AwaitingInput,
            "needs_attention" => Self::NeedsAttention,
            "error"           => Self::Error,
            "ended"           => Self::Ended,
            other => return Err(Error::Config(format!("unknown session state: {other}"))),
        })
    }
}

/// Origin of a state-changing event. Stored verbatim in `events.source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Triggered by a model-chosen MCP tool call.
    Mcp,
    /// Fired by Claude Code via `/hooks/ingest`.
    Hook,
    /// Triggered by a dashboard / CLI action.
    Operator,
    /// Synthesized by GalvoLite itself (orphan reconciler, signal bridge,
    /// startup recovery markers).
    System,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Mcp      => "mcp",
            Self::Hook     => "hook",
            Self::Operator => "operator",
            Self::System   => "system",
        })
    }
}

impl FromStr for Source {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "mcp"      => Self::Mcp,
            "hook"     => Self::Hook,
            "operator" => Self::Operator,
            "system"   => Self::System,
            other => return Err(Error::Config(format!("unknown event source: {other}"))),
        })
    }
}

/// Input to [`apply_event`]. The funnel writes one `events` row per call
/// (skipped if `dedup_key` already exists).
#[derive(Debug, Clone)]
pub struct EventInput {
    pub source: Source,
    /// Free-form event kind, e.g. `"hook.SessionStart"`, `"mcp.set_role"`,
    /// `"signal.STATUS:WORKING"`, `"system.recovered"`. The convention is
    /// `"<source>.<name>"` but the funnel does not enforce it — kind is
    /// the dimension `read_events`/dashboard filters on, so use whatever
    /// the consumer expects.
    pub kind: String,
    pub payload: serde_json::Value,
    /// Optional UNIQUE key. If present and a prior row carries the same
    /// value, the funnel returns `deduped = true` without writing.
    pub dedup_key: Option<String>,
}

/// Result of a successful `apply_event` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOutcome {
    /// Rowid of the new `events` row, or the existing row if `deduped`.
    pub event_id: i64,
    /// State the session was in immediately before the event landed
    /// (`None` if this is the first event for a brand-new row).
    pub prev_state: Option<SessionState>,
    /// State the session is in after the event was applied. Equals
    /// `prev_state` for events that don't change state (logging-only).
    pub next_state: SessionState,
    /// `true` if the event was a no-op because `dedup_key` matched a
    /// prior row.
    pub deduped: bool,
}

/// Single funnel for every state-changing event. Runs in one SQLite
/// transaction containing `(events row append, sessions row update,
/// last_event_ts bump)` so the events log is the authoritative pre-image
/// of every state change.
///
/// Returns [`EventOutcome`] describing prev/next state and whether the
/// call was deduped. Callers must not write `sessions.state` directly;
/// per spec §8.3 this is the only legal mutation path.
pub fn apply_event(
    db: &crate::db::Db,
    session_id: uuid::Uuid,
    input: EventInput,
) -> Result<EventOutcome> {
    let mut conn = db.pool().get()?;
    let tx = conn.transaction()?;

    // 1. Read current state (for transition computation).
    let current_str: String = tx
        .query_row(
            "SELECT state FROM sessions WHERE id = ?1",
            [session_id.to_string()],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Error::SessionNotFound(session_id),
            other => Error::Db(other),
        })?;
    let current = SessionState::from_str(&current_str)?;

    // 2. Reject transitions out of terminal states.
    if current.is_terminal() {
        return Err(Error::TerminalState {
            session_id: session_id.to_string(),
            kind: input.kind.clone(),
        });
    }

    // 3. Compute next state from (current, kind). For payload-sensitive
    // events (currently `mcp.status_report`) override the table result based
    // on the payload's discriminator. Keeps the declarative arm count low.
    let mut next = transition(current, &input.kind);
    if input.kind == "mcp.status_report" && !current.is_terminal() {
        match input.payload.get("kind").and_then(|v| v.as_str()) {
            Some("blocked") => next = SessionState::NeedsAttention,
            Some("complete") => next = SessionState::Idle,
            Some("working") => next = SessionState::Working,
            _ => {}
        }
    }

    // 4. Append events row (or short-circuit on dedup hit).
    let payload_str = serde_json::to_string(&input.payload)?;
    let (event_id, deduped) = crate::db::Db::append_event_in_tx(
        &tx,
        session_id,
        &input.source.to_string(),
        &input.kind,
        &payload_str,
        Some(&current.to_string()),
        Some(&next.to_string()),
        input.dedup_key.as_deref(),
    )?;
    if deduped {
        tx.commit()?;
        return Ok(EventOutcome {
            event_id,
            prev_state: Some(current),
            next_state: current,
            deduped: true,
        });
    }

    // 5. Update sessions row state + last_event_ts.
    tx.execute(
        "UPDATE sessions
         SET    state         = ?1,
                last_event_ts = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE  id = ?2",
        rusqlite::params![next.to_string(), session_id.to_string()],
    )?;

    // 5a. Side-effect column writes that must land in the same transaction
    // as the event row to preserve §8.3 atomicity. Each branch is keyed off
    // `kind` so the funnel stays declarative (mirrors `transition` above).
    apply_column_side_effects(&tx, session_id, &input)?;

    // For terminal next_states, stamp ended_at if not already set AND revoke
    // the API token (spec §7.2: the per-session bearer is invalidated the
    // moment a session reaches a terminal state, so any in-flight curl with
    // a stale token gets 401). Both updates land in the same transaction as
    // the state change (spec §8.3 atomicity).
    if matches!(next, SessionState::Ended | SessionState::Error) {
        tx.execute(
            "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id = ?1 AND ended_at IS NULL",
            [session_id.to_string()],
        )?;
        tx.execute(
            "UPDATE sessions SET api_token = NULL WHERE id = ?1",
            [session_id.to_string()],
        )?;
    }

    tx.commit()?;
    Ok(EventOutcome {
        event_id,
        prev_state: Some(current),
        next_state: next,
        deduped: false,
    })
}

/// In-transaction column writes that piggy-back on the event funnel. Reads
/// the relevant fields out of `input.payload` and updates the appropriate
/// session column. Unrecognized kinds are no-ops.
fn apply_column_side_effects(
    tx: &rusqlite::Transaction<'_>,
    session_id: uuid::Uuid,
    input: &EventInput,
) -> Result<()> {
    let sid = session_id.to_string();
    match input.kind.as_str() {
        "mcp.set_plan" => {
            if let Some(md) = input.payload.get("markdown").and_then(|v| v.as_str()) {
                tx.execute(
                    "UPDATE sessions SET plan_markdown = ?1 WHERE id = ?2",
                    rusqlite::params![md, sid],
                )?;
                let parsed = crate::plan_steps::parse_plan_markdown(md);
                crate::db::Db::delete_plan_steps_for_session_in_tx(tx, session_id)?;
                for s in &parsed {
                    crate::db::Db::upsert_plan_step_in_tx(
                        tx,
                        session_id,
                        &s.step_hash,
                        s.ordinal,
                        &s.title,
                        s.status.as_db_str(),
                        None,
                    )?;
                }
            }
        }
        "mcp.plan_check_step" => {
            // Resolve target by step_hash (preferred) or by hashing title.
            let hash: Option<String> = input.payload
                .get("step_hash").and_then(|v| v.as_str()).map(str::to_string)
                .or_else(|| input.payload.get("title").and_then(|v| v.as_str())
                    .map(crate::plan_steps::step_hash_for));
            let Some(hash) = hash else { return Ok(()); };
            let status_in = input.payload.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
            let status = crate::plan_steps::PlanStepStatus::from_input_str(status_in)
                .map(|s| s.as_db_str().to_string())
                .unwrap_or_else(|| "pending".to_string());
            let note = input.payload.get("note").and_then(|v| v.as_str());
            let _ = crate::db::Db::set_plan_step_status_in_tx(
                tx, session_id, &hash, &status, note,
            )?;
        }
        "mcp.set_seed" => {
            if let Some(md) = input.payload.get("markdown").and_then(|v| v.as_str()) {
                tx.execute(
                    "UPDATE sessions SET seed_markdown = ?1 WHERE id = ?2",
                    rusqlite::params![md, sid],
                )?;
            }
        }
        "mcp.set_role" => {
            if let Some(role) = input.payload.get("role").and_then(|v| v.as_str()) {
                tx.execute(
                    "UPDATE sessions SET role = ?1 WHERE id = ?2",
                    rusqlite::params![role, sid],
                )?;
            }
        }
        "mcp.write_handoff" => {
            if let Some(p) = input.payload.get("path").and_then(|v| v.as_str()) {
                tx.execute(
                    "UPDATE sessions SET handoff_path = ?1 WHERE id = ?2",
                    rusqlite::params![p, sid],
                )?;
            }
        }
        "mcp.request_attention" => {
            if let Some(reason) = input.payload.get("reason").and_then(|v| v.as_str()) {
                tx.execute(
                    "UPDATE sessions SET last_status_note = ?1 WHERE id = ?2",
                    rusqlite::params![reason, sid],
                )?;
            }
        }
        "mcp.status_report" => {
            if let Some(note) = input.payload.get("note").and_then(|v| v.as_str()) {
                tx.execute(
                    "UPDATE sessions SET last_status_note = ?1 WHERE id = ?2",
                    rusqlite::params![note, sid],
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Like [`apply_event`] but also publishes `SessionEventAppended` and
/// (when state changed) `SessionStateChanged` to the dashboard event bus.
/// Production callers (hook handlers, MCP tools, signal pump, lifecycle)
/// should prefer this over the bare [`apply_event`].
///
/// Order of published events matches the order of consumer interest:
/// `SessionEventAppended` first (the underlying row), then
/// `SessionStateChanged` (derived state mutation). Deduped events publish
/// nothing — the row was a no-op so subscribers should not see a phantom.
pub fn apply_event_with_bus(
    db: &crate::db::Db,
    session_id: uuid::Uuid,
    bus: &crate::api::events::EventBroadcaster,
    input: EventInput,
) -> Result<EventOutcome> {
    let kind = input.kind.clone();
    let source_str = input.source.to_string();
    let payload = input.payload.clone();

    // Sub-plan 5 Task 3: snapshot prior plan_steps for diff before write.
    // Only `mcp.set_plan` mutates the table, so other kinds skip the read.
    // Propagate DB errors here with `?` — the event has not yet committed,
    // so failing is safe and preferable to silently treating every step as new.
    let prior_plan_steps: Option<Vec<crate::plan_steps::DbPlanStep>> =
        if kind == "mcp.set_plan" {
            Some(db.read_plan_steps_raw(session_id)?)
        } else {
            None
        };

    // Sub-plan 5 Task 4: snapshot prior status of the targeted plan_step row
    // for `mcp.plan_check_step`. Resolved by step_hash (preferred) or by
    // hashing the title. Missing rows are a normal no-op (the corresponding
    // test asserts so); a real DB error is logged but does not block the
    // event from committing.
    let prior_step_status: Option<(String, String)> = if kind == "mcp.plan_check_step" {
        let hash = input
            .payload
            .get("step_hash")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| {
                input
                    .payload
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(crate::plan_steps::step_hash_for)
            });
        hash.and_then(|h| match db.read_plan_step_status(session_id, &h) {
            Ok(opt) => opt.map(|s| (h, s)),
            Err(e) => {
                tracing::warn!(
                    "plan_check_step prior-status read failed for {session_id}/{h}: {e}"
                );
                None
            }
        })
    } else {
        None
    };

    let outcome = apply_event(db, session_id, input)?;
    if !outcome.deduped {
        let ts = chrono::Utc::now().to_rfc3339();
        bus.publish(crate::api::events::DomainEvent::SessionEventAppended {
            session_id,
            event_id: outcome.event_id,
            kind: kind.clone(),
            source: source_str,
            ts: ts.clone(),
            payload: payload.clone(),
        });
        if Some(outcome.next_state) != outcome.prev_state {
            bus.publish(crate::api::events::DomainEvent::SessionStateChanged {
                session_id,
                prev: outcome.prev_state,
                next: outcome.next_state,
                last_event_ts: ts,
            });
        }
        // Sub-plan 5 Task 3: emit `SessionPlanReplaced` and per-step
        // `SessionPlanStepChanged` events whenever `set_plan` rewrites the
        // plan_steps table. Diff is computed against the pre-event snapshot
        // captured above (post-event read would always show zero changes
        // because we delete-then-reinsert).
        //
        // The event was already committed above, so failures here must NOT
        // return an error to the caller (that would falsely imply the event
        // didn't land). Log and skip the bus publish instead.
        if kind == "mcp.set_plan" {
            match db.read_plan_steps_raw(session_id) {
                Ok(next_rows) => {
                    let json_steps = match db.read_plan_steps(session_id) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(
                                "set_plan publish: read_plan_steps failed for {session_id}: {e}"
                            );
                            Vec::new()
                        }
                    };
                    bus.publish(crate::api::events::DomainEvent::SessionPlanReplaced {
                        session_id,
                        steps: serde_json::Value::Array(json_steps),
                    });
                    let prior = prior_plan_steps.unwrap_or_default();
                    let next_inputs: Vec<crate::plan_steps::PlanStepInput> = next_rows
                        .into_iter()
                        .map(|r| crate::plan_steps::PlanStepInput {
                            step_hash: r.step_hash,
                            title:     r.title,
                            status:    crate::plan_steps::PlanStepStatus::from_input_str(&r.status)
                                .unwrap_or(crate::plan_steps::PlanStepStatus::Pending),
                            ordinal:   r.ordinal,
                        })
                        .collect();
                    let diff = crate::plan_steps::diff_plan_steps(&prior, &next_inputs);
                    for c in diff.changed {
                        bus.publish(crate::api::events::DomainEvent::SessionPlanStepChanged {
                            session_id,
                            step_hash:   c.step_hash,
                            prev_status: c.prev_status,
                            next_status: c.next_status,
                            note:        None,
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "set_plan publish skipped: read_plan_steps_raw failed for {session_id}: {e}"
                    );
                }
            }
        }
        // Sub-plan 5 Task 4: emit `SessionPlanStepChanged` when a single-step
        // status update actually changed the row's status. Skip when the row
        // didn't exist before (`prior_step_status == None`) or the status is
        // unchanged.
        if kind == "mcp.plan_check_step" {
            let new_status_in = payload.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
            let new_status = crate::plan_steps::PlanStepStatus::from_input_str(new_status_in)
                .map(|s| s.as_db_str().to_string())
                .unwrap_or_else(|| "pending".into());
            if let Some((hash, prev)) = prior_step_status {
                if prev != new_status {
                    bus.publish(crate::api::events::DomainEvent::SessionPlanStepChanged {
                        session_id,
                        step_hash:   hash,
                        prev_status: prev,
                        next_status: new_status,
                        note:        payload.get("note").and_then(|v| v.as_str()).map(str::to_string),
                    });
                }
            }
        }
    }
    Ok(outcome)
}

/// Transition table from spec §8.6 plus signal-bridge rows. Anything not
/// matched here is treated as logging-only (state unchanged).
fn transition(current: SessionState, kind: &str) -> SessionState {
    use SessionState::*;
    match (current, kind) {
        // Lifecycle (used by SessionManager via Task 7 + Sub-plan 2 hooks).
        // Per spec §8.6, system.session_started → Idle. The UserPromptSubmit
        // hook (Sub-plan 2 Task 8) now drives Idle → Working naturally, so
        // we no longer need the Sub-plan 1 deviation that landed straight
        // in Working.
        (Starting, "system.session_started") | (Starting, "hook.SessionStart") => Idle,
        (s,        "system.session_ended")   if !s.is_terminal()               => Ended,
        (s,        "hook.SessionEnd")        if !s.is_terminal()               => current,
        (s,        "system.session_errored") if !s.is_terminal()               => Error,
        (_,        "system.session_orphaned")                                  => Error,

        // Signal bridge (System events emitted by SignalPump in Task 8).
        (Idle,           "signal.STATUS:WORKING")    => Working,
        (Working,        "signal.STATUS:WORKING")    => Working,
        (NeedsAttention, "signal.STATUS:WORKING")    => Working,
        (AwaitingInput,  "signal.STATUS:WORKING")    => Working,
        (Starting,       "signal.STATUS:WORKING")    => Working,

        (Working,        "signal.STATUS:COMPLETE")   => Idle,
        (NeedsAttention, "signal.STATUS:COMPLETE")   => Idle,
        (AwaitingInput,  "signal.STATUS:COMPLETE")   => Idle,

        (Working,        "signal.ATTENTION:NEEDED")  => NeedsAttention,
        (Idle,           "signal.ATTENTION:NEEDED")  => NeedsAttention,
        (AwaitingInput,  "signal.ATTENTION:NEEDED")  => NeedsAttention,

        // Turn-tracking hooks (Sub-plan 2 Task 8). UserPromptSubmit drives
        // Idle → Working naturally; the spec §8.6 normalization (Task 3)
        // will land system.session_started → Idle so this is the canonical
        // path for entering Working from outside the signal bridge.
        (Idle,           "hook.UserPromptSubmit")    => Working,
        (AwaitingInput,  "hook.UserPromptSubmit")    => Working,
        (NeedsAttention, "hook.UserPromptSubmit")    => Working,
        (Working,        "hook.UserPromptSubmit")    => Working,
        // hook.Stop is logged but doesn't transition; on_stop emits
        // system.stop_max_retries to flip Working → Idle when the limit
        // is reached.
        (Working,        "system.stop_max_retries")  => Idle,
        // The 1- and 2-Stop watchdog (turn::on_stop) emits
        // system.awaiting_input from whatever state the session was in when
        // Stop arrived — usually Working, sometimes NeedsAttention. Without
        // these arms the watchdog would no-op (the spawned task spins, fires,
        // and finds no transition match).
        (Working,        "system.awaiting_input")    => AwaitingInput,
        (NeedsAttention, "system.awaiting_input")    => AwaitingInput,
        (Idle,           "system.awaiting_input")    => AwaitingInput,

        // Observation hooks (Sub-plan 2 Task 9). PostToolUse(Bash) for
        // `git push` raises attention via `system.attention_needed`; the
        // hook itself doesn't transition state.
        (s,              "system.attention_needed")  if !s.is_terminal() => NeedsAttention,

        // MCP tools (Sub-plan 3 §5.1 status & lifecycle).
        // request_attention always escalates (no-op if already NeedsAttention).
        // mcp.status_report's per-kind state effect is computed in apply_event
        // because it depends on the payload's `kind` field (declarative-table
        // arms key only on event kind, not payload).
        (s, "mcp.request_attention") if !s.is_terminal() => NeedsAttention,

        // MCP tools §5.2 self-control (computed in apply_event for the ones
        // that need state changes — `mcp.pause_self` → AwaitingInput,
        // `mcp.request_end` → Ended). Logged-only kinds are listed below.
        (s, "mcp.pause_self")    if !s.is_terminal() => AwaitingInput,
        (_, "mcp.request_end")                       => Ended,

        // MCP tools §5.3/§5.4 cross-session + operator. All logging-only:
        //  - `mcp.send_to_session` is recorded against the TARGET session so
        //    it shows in the receiver's event log; the receiver's actual state
        //    change lands via its own UserPromptSubmit hook (Sub-plan 2).
        //  - `mcp.ask_operator` / `mcp.notify_operator` are pure side-effects
        //    in degraded mode (Sub-plan 4 wires the dashboard WS round-trip).
        (s, "mcp.send_to_session") if !s.is_terminal() => s,
        (s, "mcp.ask_operator")    if !s.is_terminal() => s,
        (s, "mcp.notify_operator") if !s.is_terminal() => s,

        // Everything else logs but doesn't change state. Includes:
        //  - signal.HANDOFF:READY / HANDOFF:BLOCKED / PLAN:READY / SEED:NEW /
        //    ROLE:SET / ATTENTION:CONTEXT — model-side annotations
        //  - mcp.* tool calls (Sub-plan 3)
        //  - hook.PreCompact / hook.UserPromptSubmit / hook.Stop /
        //    hook.PostToolUse / hook.Notification (Sub-plan 2)
        //  - operator.* (Sub-plan 4)
        _ => current,
    }
}
