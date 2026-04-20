//! `PostToolUse(Bash)`, `Notification` handlers (Task 9 stubs).

use crate::db::Db;
use uuid::Uuid;

pub fn on_post_tool_use(_db: &Db, _id: Uuid, _payload: &serde_json::Value) -> Option<String> {
    None
}

pub fn on_notification(_db: &Db, _id: Uuid, _payload: &serde_json::Value) -> Option<String> {
    None
}
