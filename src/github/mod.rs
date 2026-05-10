//! Plan 10a — GitHub integration layer.
//!
//! Read-only PAT-based access via `octocrab`. The client is constructed once
//! at server startup from `GITHUB_TOKEN`; the connected-user cache lives on
//! the API state and is shared with all handlers.

pub mod client;
pub mod push;

pub use client::{
    Client, IssueSummary, PrInfo, PrReviewsBundle, PrSummary, ReviewCommentSummary, ReviewSummary,
    User,
};
pub use push::PushError;
