//! `UserPromptSubmit`, `Stop`, `PreCompact` handlers (Task 8 stubs).

use crate::db::Db;
use uuid::Uuid;

pub fn on_user_prompt_submit(_db: &Db, _id: Uuid, _payload: &serde_json::Value) -> Option<String> {
    None
}

pub fn on_stop(_db: &Db, _id: Uuid) -> Option<String> {
    None
}

pub fn on_pre_compact(_db: &Db, _id: Uuid) -> Option<String> {
    None
}
