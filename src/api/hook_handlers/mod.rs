//! Per-hook semantics for `/hooks/ingest`.
//!
//! Each submodule owns a small set of hook kinds and exposes pure functions
//! (`db: &Db, session_id, payload`) that the dispatch table in
//! `src/api/hooks.rs` calls after `apply_event` writes the row.

pub mod lifecycle;
pub mod observe;
pub mod turn;
