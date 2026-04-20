//! Declarative MCP tool registry.
//!
//! Each `ToolDef` pairs an MCP name + schema + tier tag + async handler.
//! Built once at startup; `tools/list` and `tools/call` route through it.

use crate::session_manager::SessionManager;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

pub struct ToolCtx {
    pub mgr: Arc<SessionManager>,
    pub session_id: Uuid,
}

pub type ToolHandler = Box<
    dyn Fn(ToolCtx, Value) -> Pin<Box<dyn Future<Output = ToolOutcome> + Send>>
        + Send
        + Sync,
>;

pub enum ToolOutcome {
    Ok(Value),
    /// Handler completed but produced an application-level error. Wraps as
    /// `ToolsCallResult { isError: true }` in the JSON-RPC response so the
    /// model can read and adapt. Protocol errors use JSON-RPC codes instead.
    AppError(String),
    /// Argument validation failure — surfaces as JSON-RPC -32602.
    InvalidParams(String),
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Tier {
    /// Self-contained — works against DB only.
    SelfContained,
    /// Drives a sibling PTY. Requires the target session to be alive.
    CrossSessionPty,
    /// Needs the dashboard to function. Degrades until Sub-plan 4 wires the
    /// WS surface.
    DashboardDegraded,
    /// Reserved name; handler returns `not_implemented`. Plans 4-6 fill in.
    ReservedStub,
}

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub tier: Tier,
    pub handler: ToolHandler,
}

#[derive(Clone)]
pub struct ToolRegistry {
    inner: Arc<HashMap<&'static str, Arc<ToolDef>>>,
}

impl ToolRegistry {
    pub fn build() -> Self {
        let defs = crate::mcp::tools::all();
        let map = defs
            .into_iter()
            .map(|d| (d.name, Arc::new(d)))
            .collect::<HashMap<_, _>>();
        Self {
            inner: Arc::new(map),
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<ToolDef>> {
        self.inner.get(name).cloned()
    }

    pub fn list_descriptors(&self) -> Vec<Value> {
        let mut v: Vec<_> = self
            .inner
            .values()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "description": d.description,
                    "inputSchema": d.input_schema,
                })
            })
            .collect();
        v.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap()
                .cmp(b["name"].as_str().unwrap())
        });
        v
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::build()
    }
}
