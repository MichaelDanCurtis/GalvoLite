//! Kimi K2 HTTP client (OpenAI-compatible).
//! Endpoint: <base_url>/chat/completions

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum KimiError {
    #[error("transient: {0}")] Transient(String),
    #[error("permanent: {0}")] Permanent(String),
    #[error("malformed json: {0}")] Malformed(String),
}

pub struct KimiClient {
    api_key: String,
    base_url: String,
    model: String,
    http: reqwest::Client,
}

#[derive(Serialize)]
struct ChatReq<'a> {
    model: &'a str,
    messages: Vec<ChatMsg<'a>>,
    temperature: f32,
}

#[derive(Serialize)]
struct ChatMsg<'a> { role: &'a str, content: &'a str }

#[derive(Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}
#[derive(Deserialize)]
struct Choice { message: ChoiceMessage }
#[derive(Deserialize)]
struct ChoiceMessage { content: String }
#[derive(Deserialize)]
struct Usage { total_tokens: u64 }

impl KimiClient {
    pub fn new(api_key: &str, base_url: &str, model: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .user_agent("KimiCLI/1.41.0")
                .build().unwrap(),
        }
    }

    pub async fn chat(&self, system: &str, user: &str) -> Result<(String, u64), KimiError> {
        let body = ChatReq {
            model: &self.model,
            messages: vec![
                ChatMsg { role: "system", content: system },
                ChatMsg { role: "user",   content: user   },
            ],
            temperature: 0.2,
        };
        let url = format!("{}/chat/completions", self.base_url);
        let resp = self.http.post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await
            .map_err(|e| KimiError::Transient(e.to_string()))?;
        let status = resp.status();
        if status.is_server_error() {
            return Err(KimiError::Transient(format!("{status}")));
        }
        if !status.is_success() {
            return Err(KimiError::Permanent(format!("{status}")));
        }
        let parsed: ChatResp = resp.json().await
            .map_err(|e| KimiError::Malformed(e.to_string()))?;
        let text = parsed.choices.into_iter().next()
            .ok_or_else(|| KimiError::Malformed("no choices".into()))?
            .message.content;
        let tokens = parsed.usage.map(|u| u.total_tokens).unwrap_or(0);
        Ok((text, tokens))
    }
}
