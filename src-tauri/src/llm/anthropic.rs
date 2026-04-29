//! Anthropic Messages API adapter.
//!
//! Uses tool-use mode and applies prompt caching to the stable portion of
//! the system prompt and the tools array — those rarely change between
//! turns and account for the bulk of input tokens.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ChatRequest, ChatResponse, ContentBlock, LLMProvider, Role, StopReason, Usage};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Default model. Haiku is ~4–5× cheaper than Sonnet and its tool-use
/// accuracy on Mr. Moneypenny's structured workload (`add_expense`,
/// `summarize_period`, etc.) is more than sufficient. Users can override
/// via the saved `anthropic_model` setting.
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::with_options(api_key, DEFAULT_MODEL, DEFAULT_BASE_URL)
    }

    pub fn with_options(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self> {
        let client = Client::builder()
            .user_agent("moneypenny/0.1")
            .build()
            .context("building reqwest client")?;
        Ok(Self {
            client,
            api_key: api_key.into(),
            base_url: base_url.into(),
            model: model.into(),
        })
    }
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let body = build_body(&self.model, &request);
        let url = format!("{}/v1/messages", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("anthropic POST /v1/messages")?;

        let status = resp.status();
        let body_text = resp.text().await.context("reading anthropic response")?;
        if !status.is_success() {
            return Err(anyhow!(
                "anthropic API error {}: {}",
                status,
                truncate(&body_text, 1024)
            ));
        }

        let parsed: ApiResponse = serde_json::from_str(&body_text).with_context(|| {
            format!("parsing anthropic response: {}", truncate(&body_text, 512))
        })?;
        Ok(parsed.into_chat_response())
    }
}

// ---------------------------------------------------------------------
// Wire formats.
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ApiBody<'a> {
    model: &'a str,
    max_tokens: u32,
    system: Vec<SystemBlock<'a>>,
    tools: Vec<ToolBlock<'a>>,
    messages: Vec<ApiMessage>,
}

#[derive(Debug, Serialize)]
struct SystemBlock<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Serialize)]
struct ToolBlock<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a Value,
    /// Only set on the last tool to cache the entire tools block.
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Serialize, Clone, Copy)]
struct CacheControl {
    #[serde(rename = "type")]
    kind: &'static str,
}

const EPHEMERAL: CacheControl = CacheControl { kind: "ephemeral" };

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    content: Vec<ApiContentOut>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ApiContentOut {
    Text {
        #[serde(rename = "type")]
        kind: &'static str,
        text: String,
    },
    ToolUse {
        #[serde(rename = "type")]
        kind: &'static str,
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        #[serde(rename = "type")]
        kind: &'static str,
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

fn build_body<'a>(model: &'a str, req: &'a ChatRequest) -> ApiBody<'a> {
    // System: stable block is cached, volatile is not.
    let mut system = vec![SystemBlock {
        kind: "text",
        text: &req.system_prompt.stable,
        cache_control: Some(EPHEMERAL),
    }];
    if !req.system_prompt.volatile.is_empty() {
        system.push(SystemBlock {
            kind: "text",
            text: &req.system_prompt.volatile,
            cache_control: None,
        });
    }

    // Tools: cache the whole array via cache_control on the last entry.
    let last_idx = req.tools.len().saturating_sub(1);
    let tools = req
        .tools
        .iter()
        .enumerate()
        .map(|(i, t)| ToolBlock {
            name: &t.name,
            description: &t.description,
            input_schema: &t.input_schema,
            cache_control: if i == last_idx { Some(EPHEMERAL) } else { None },
        })
        .collect();

    let messages = req
        .messages
        .iter()
        .map(|m| ApiMessage {
            role: match m.role {
                Role::User => "user".into(),
                Role::Assistant => "assistant".into(),
            },
            content: m.content.iter().map(message_block_to_api).collect(),
        })
        .collect();

    ApiBody {
        model,
        max_tokens: req.max_tokens,
        system,
        tools,
        messages,
    }
}

fn message_block_to_api(block: &ContentBlock) -> ApiContentOut {
    match block {
        ContentBlock::Text(t) => ApiContentOut::Text {
            kind: "text",
            text: t.clone(),
        },
        ContentBlock::ToolUse { id, name, input } => ApiContentOut::ToolUse {
            kind: "tool_use",
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => ApiContentOut::ToolResult {
            kind: "tool_result",
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
    }
}

// ---------------------------------------------------------------------
// Response.
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ApiContentIn>,
    stop_reason: Option<String>,
    #[serde(default)]
    usage: ApiUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ApiContentIn {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Anthropic also can emit thinking blocks; we ignore them.
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize, Default)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

impl ApiResponse {
    fn into_chat_response(self) -> ChatResponse {
        let stop_reason = match self.stop_reason.as_deref() {
            Some("end_turn") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::Other,
        };
        let content = self
            .content
            .into_iter()
            .filter_map(|c| match c {
                ApiContentIn::Text { text } => Some(ContentBlock::Text(text)),
                ApiContentIn::ToolUse { id, name, input } => {
                    Some(ContentBlock::ToolUse { id, name, input })
                }
                ApiContentIn::Other => None,
            })
            .collect();
        let usage = Usage {
            input_tokens: self.usage.input_tokens,
            output_tokens: self.usage.output_tokens,
            cache_creation_input_tokens: self.usage.cache_creation_input_tokens,
            cache_read_input_tokens: self.usage.cache_read_input_tokens,
        };
        ChatResponse {
            stop_reason,
            content,
            usage,
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

// Note: provider tests for Anthropic require a live API key or HTTP
// fixtures. Round-trip tests live in tests/integration_dispatcher.rs
// with a stub LLMProvider; this module is exercised end-to-end via the
// `tauri dev` smoke test.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{tools::all_tools, Message, SystemPrompt};

    #[test]
    fn body_caches_stable_system_and_last_tool() {
        let req = ChatRequest {
            system_prompt: SystemPrompt {
                stable: "STATIC".into(),
                volatile: "VOLATILE".into(),
            },
            messages: vec![Message::user_text("$5 coffee")],
            tools: all_tools(),
            max_tokens: 1024,
        };
        let body = build_body("claude-haiku-4-5-20251001", &req);

        // Stable system block has cache_control; volatile does not.
        assert_eq!(body.system.len(), 2);
        assert!(body.system[0].cache_control.is_some());
        assert!(body.system[1].cache_control.is_none());

        // Last tool has cache_control; preceding tools do not.
        let n = body.tools.len();
        for (i, t) in body.tools.iter().enumerate() {
            if i == n - 1 {
                assert!(t.cache_control.is_some(), "last tool should cache");
            } else {
                assert!(t.cache_control.is_none(), "non-last tool should not cache");
            }
        }
    }
}
