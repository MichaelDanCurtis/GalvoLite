//! GalvoLite ritual engine — library crate.
//!
//! See `docs/superpowers/specs/2026-04-15-galvolite-design.md` for the product spec
//! and `docs/superpowers/plans/2026-04-15-plan-1a-ritual-engine.md` for this plan.

pub mod api;
pub mod auth;
pub mod claude_md;
pub mod cli;
pub mod db;
pub mod error;
pub mod handoff;
pub mod hooks;
pub mod mcp;
pub mod pty;
pub mod ritual;
pub mod session_manager;
pub mod session_registry;
pub mod plan_steps;
pub mod spawn_config;
pub mod state;
pub mod worktree;

pub use error::{Error, Result};
