//! Typed wrappers around the Telegram Bot API methods we use.
//!
//! Only the fields we actually consume are deserialized — extras are
//! ignored. The trait `TelegramApi` lets the router be tested without
//! making live HTTPS calls.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE_URL: &str = "https://api.telegram.org";

#[async_trait]
pub trait TelegramApi: Send + Sync {
    async fn get_me(&self) -> Result<User>;
    async fn get_updates(&self, offset: i64, timeout: u32) -> Result<Vec<Update>>;
    async fn send_message(&self, chat_id: i64, text: &str) -> Result<Message>;
    async fn delete_webhook(&self) -> Result<()>;
}

pub struct TelegramClient {
    http: Client,
    token: String,
    base_url: String,
}

impl TelegramClient {
    pub fn new(token: impl Into<String>) -> Result<Self> {
        Self::with_base_url(token, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .user_agent("moneypenny/0.1")
            .build()
            .context("building reqwest client")?;
        Ok(Self {
            http,
            token: token.into(),
            base_url: base_url.into(),
        })
    }

    fn url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.base_url, self.token, method)
    }

    async fn invoke<R: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        body: serde_json::Value,
    ) -> Result<R> {
        let resp = self
            .http
            .post(self.url(method))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("telegram POST {method}: {}", scrub_token(e)))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| anyhow!("reading telegram response: {}", scrub_token(e)))?;
        if !status.is_success() {
            return Err(anyhow!(
                "telegram {method} HTTP {status}: {}",
                redact_path(&truncate(&text, 512))
            ));
        }
        let parsed: ApiResponse<R> = serde_json::from_str(&text).with_context(|| {
            format!(
                "parsing telegram {method}: {}",
                redact_path(&truncate(&text, 256))
            )
        })?;
        if !parsed.ok {
            return Err(anyhow!(
                "telegram {method}: {}",
                parsed.description.unwrap_or_else(|| "unknown error".into())
            ));
        }
        parsed
            .result
            .ok_or_else(|| anyhow!("telegram {method}: missing result"))
    }
}

/// Format a `reqwest::Error` for logging without leaking the bot token.
///
/// The Telegram Bot API embeds the token in the URL path
/// (`/bot<token>/<method>`), so on transport failures (DNS, TLS, timeout,
/// redirect) `reqwest::Error::Display` may include the URL — and therefore
/// the token — in error chains that get logged via `tracing::error/warn!`.
/// We strip the URL with `Error::without_url()` first, then run a defensive
/// regex-free scrub on the resulting string in case any nested error type
/// has already stringified the URL into its own message.
fn scrub_token(e: reqwest::Error) -> String {
    redact_path(&e.without_url().to_string())
}

/// Replace any `/bot<...>/...` segment in `s` with `/bot<REDACTED>/...`.
/// Used as the inner string scrub for `scrub_token` and applied
/// defensively to response-body excerpts.
fn redact_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find("/bot") {
        out.push_str(&rest[..idx + 4]);
        rest = &rest[idx + 4..];
        match rest.find('/') {
            Some(end) => {
                out.push_str("<REDACTED>");
                rest = &rest[end..];
            }
            None => {
                out.push_str("<REDACTED>");
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[async_trait]
impl TelegramApi for TelegramClient {
    async fn get_me(&self) -> Result<User> {
        self.invoke("getMe", serde_json::json!({})).await
    }

    async fn get_updates(&self, offset: i64, timeout: u32) -> Result<Vec<Update>> {
        self.invoke(
            "getUpdates",
            serde_json::json!({
                "offset": offset,
                "timeout": timeout,
                "allowed_updates": ["message"],
            }),
        )
        .await
    }

    async fn send_message(&self, chat_id: i64, text: &str) -> Result<Message> {
        // We send plain text rather than MarkdownV2 by default to avoid
        // escape mistakes in dynamic content. Specific replies that want
        // formatting can be wrapped by formatter::escape_md_v2 first and
        // a parse_mode-enabled overload added later.
        self.invoke(
            "sendMessage",
            serde_json::json!({
                "chat_id": chat_id,
                "text": text,
                "disable_web_page_preview": true,
            }),
        )
        .await
    }

    async fn delete_webhook(&self) -> Result<()> {
        let _: bool = self
            .invoke(
                "deleteWebhook",
                serde_json::json!({ "drop_pending_updates": false }),
            )
            .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Wire types — only the fields we read.
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default = "Option::default")]
    result: Option<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub date: i64,
    pub chat: Chat,
    #[serde(default)]
    pub from: Option<User>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Update {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<Message>,
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_get_me_response() {
        let raw = r#"{"ok":true,"result":{"id":123,"is_bot":true,"first_name":"Mr Moneypenny","username":"moneypenny_bot"}}"#;
        let resp: ApiResponse<User> = serde_json::from_str(raw).unwrap();
        assert!(resp.ok);
        let user = resp.result.unwrap();
        assert_eq!(user.id, 123);
        assert_eq!(user.username.as_deref(), Some("moneypenny_bot"));
    }

    #[test]
    fn parse_get_updates_with_message() {
        let raw = r#"{
            "ok": true,
            "result": [
                {
                    "update_id": 42,
                    "message": {
                        "message_id": 1,
                        "date": 1714320000,
                        "from": {"id": 999, "is_bot": false, "first_name": "Wyatt"},
                        "chat": {"id": 999, "type": "private", "first_name": "Wyatt"},
                        "text": "$5 coffee"
                    }
                }
            ]
        }"#;
        let resp: ApiResponse<Vec<Update>> = serde_json::from_str(raw).unwrap();
        let updates = resp.result.unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_id, 42);
        let msg = updates[0].message.as_ref().unwrap();
        assert_eq!(msg.text.as_deref(), Some("$5 coffee"));
        assert_eq!(msg.chat.id, 999);
    }

    #[test]
    fn parse_error_response() {
        let raw = r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#;
        let resp: ApiResponse<User> = serde_json::from_str(raw).unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.description.as_deref(), Some("Unauthorized"));
    }

    #[test]
    fn redact_path_strips_telegram_bot_token() {
        // Single occurrence, full URL.
        let dirty = "error connecting to https://api.telegram.org/bot12345:ABCDEFG_secret-token/getUpdates: timeout";
        let cleaned = redact_path(dirty);
        assert!(!cleaned.contains("12345:ABCDEFG_secret-token"));
        assert!(cleaned.contains("/bot<REDACTED>/getUpdates"));
    }

    #[test]
    fn redact_path_handles_token_at_end_of_string() {
        let dirty = "/bot12345:secret";
        let cleaned = redact_path(dirty);
        assert_eq!(cleaned, "/bot<REDACTED>");
        assert!(!cleaned.contains("12345:secret"));
    }

    #[test]
    fn redact_path_handles_multiple_occurrences() {
        let dirty = "first: /bot111:aaa/getMe second: /bot222:bbb/sendMessage";
        let cleaned = redact_path(dirty);
        assert!(!cleaned.contains("111:aaa"));
        assert!(!cleaned.contains("222:bbb"));
        assert_eq!(
            cleaned,
            "first: /bot<REDACTED>/getMe second: /bot<REDACTED>/sendMessage"
        );
    }

    #[test]
    fn redact_path_leaves_unrelated_strings_alone() {
        let s = "this is fine: /api/users/42 and a search for botany";
        assert_eq!(redact_path(s), s);
    }
}
