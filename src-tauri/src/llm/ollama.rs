//! Ollama `/api/chat` adapter for local LLMs.
//!
//! Ollama supports tool-use via the `tools` field on `/api/chat` (Ollama
//! 0.3+). Caller is responsible for ensuring the chosen model actually
//! supports tool-use (e.g. `llama3.1`, `qwen2.5`, `mistral-nemo`).

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    ChatRequest, ChatResponse, ContentBlock, LLMProvider, Message, Role, StopReason, Usage,
};

const DEFAULT_BASE_URL: &str = "http://localhost:11434";

pub struct OllamaProvider {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub fn new(model: impl Into<String>) -> Result<Self> {
        Self::with_base_url(model, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(model: impl Into<String>, base_url: impl Into<String>) -> Result<Self> {
        let client = Client::builder()
            .user_agent("moneypenny/0.1")
            .build()
            .context("building reqwest client")?;
        Ok(Self {
            client,
            base_url: base_url.into(),
            model: model.into(),
        })
    }
}

#[async_trait]
impl LLMProvider for OllamaProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let body = build_body(&self.model, &request);
        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("ollama POST /api/chat")?;

        let status = resp.status();
        let body_text = resp.text().await.context("reading ollama response")?;
        if !status.is_success() {
            return Err(anyhow!(
                "ollama API error {}: {}",
                status,
                truncate(&body_text, 1024)
            ));
        }

        let parsed: ApiResponse = serde_json::from_str(&body_text)
            .with_context(|| format!("parsing ollama response: {}", truncate(&body_text, 512)))?;
        Ok(parsed.into_chat_response())
    }
}

// ---------------------------------------------------------------------
// Wire formats.
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ApiBody<'a> {
    model: &'a str,
    messages: Vec<ApiMessage>,
    tools: Vec<OllamaTool<'a>>,
    stream: bool,
    options: ApiOptions,
}

#[derive(Debug, Serialize)]
struct ApiOptions {
    num_predict: i32,
}

#[derive(Debug, Serialize)]
struct OllamaTool<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    function: OllamaFunction<'a>,
}

#[derive(Debug, Serialize)]
struct OllamaFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a Value,
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<OllamaToolCallOut>,
}

#[derive(Debug, Serialize)]
struct OllamaToolCallOut {
    function: OllamaFunctionCallOut,
}

#[derive(Debug, Serialize)]
struct OllamaFunctionCallOut {
    name: String,
    arguments: Value,
}

fn build_body<'a>(model: &'a str, req: &'a ChatRequest) -> ApiBody<'a> {
    let mut messages: Vec<ApiMessage> = Vec::with_capacity(req.messages.len() + 1);

    // Ollama only takes a single system message; concatenate stable + volatile.
    let mut system_combined = req.system_prompt.stable.clone();
    if !req.system_prompt.volatile.is_empty() {
        system_combined.push_str("\n\n");
        system_combined.push_str(&req.system_prompt.volatile);
    }
    messages.push(ApiMessage {
        role: "system".into(),
        content: system_combined,
        tool_calls: Vec::new(),
    });

    for m in &req.messages {
        messages.push(serialize_message(m));
    }

    let tools = req
        .tools
        .iter()
        .map(|t| OllamaTool {
            kind: "function",
            function: OllamaFunction {
                name: &t.name,
                description: &t.description,
                parameters: &t.input_schema,
            },
        })
        .collect();

    ApiBody {
        model,
        messages,
        tools,
        stream: false,
        options: ApiOptions {
            num_predict: req.max_tokens as i32,
        },
    }
}

fn serialize_message(m: &Message) -> ApiMessage {
    let mut text_chunks: Vec<String> = Vec::new();
    let mut tool_calls: Vec<OllamaToolCallOut> = Vec::new();
    for block in &m.content {
        match block {
            ContentBlock::Text(t) => text_chunks.push(t.clone()),
            ContentBlock::ToolUse {
                id: _, name, input, ..
            } => {
                tool_calls.push(OllamaToolCallOut {
                    function: OllamaFunctionCallOut {
                        name: name.clone(),
                        arguments: input.clone(),
                    },
                });
            }
            ContentBlock::ToolResult {
                content, is_error, ..
            } => {
                // Ollama doesn't have tool_result blocks; surface the
                // result back to the model as a `tool` role message.
                let prefix = if *is_error { "[error] " } else { "" };
                text_chunks.push(format!("{prefix}{content}"));
            }
        }
    }
    let role = match m.role {
        Role::User => {
            // If the message carries a tool_result, Ollama expects role="tool".
            if m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
            {
                "tool".into()
            } else {
                "user".into()
            }
        }
        Role::Assistant => "assistant".into(),
    };
    ApiMessage {
        role,
        content: text_chunks.join("\n"),
        tool_calls,
    }
}

// ---------------------------------------------------------------------
// Response.
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ApiResponse {
    message: ApiResponseMessage,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
}

#[derive(Debug, Deserialize)]
struct ApiResponseMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<OllamaToolCallIn>,
}

#[derive(Debug, Deserialize)]
struct OllamaToolCallIn {
    function: OllamaFunctionCallIn,
}

#[derive(Debug, Deserialize)]
struct OllamaFunctionCallIn {
    name: String,
    arguments: Value,
}

impl ApiResponse {
    fn into_chat_response(self) -> ChatResponse {
        let mut content: Vec<ContentBlock> = Vec::new();
        if !self.message.content.is_empty() {
            content.push(ContentBlock::Text(self.message.content));
        }
        let mut had_tool = false;
        for (i, tc) in self.message.tool_calls.into_iter().enumerate() {
            had_tool = true;
            // Ollama doesn't return an id; synthesize a stable one.
            content.push(ContentBlock::ToolUse {
                id: format!("ollama_tu_{i}"),
                name: tc.function.name,
                input: tc.function.arguments,
            });
        }

        let stop_reason = if had_tool {
            StopReason::ToolUse
        } else {
            match self.done_reason.as_deref() {
                Some("stop") => StopReason::EndTurn,
                Some("length") => StopReason::MaxTokens,
                _ => StopReason::Other,
            }
        };

        ChatResponse {
            stop_reason,
            content,
            usage: Usage {
                input_tokens: self.prompt_eval_count,
                output_tokens: self.eval_count,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{tools::all_tools, SystemPrompt};

    #[test]
    fn body_concatenates_system_into_one_message() {
        let req = ChatRequest {
            system_prompt: SystemPrompt {
                stable: "STATIC".into(),
                volatile: "VOLATILE".into(),
            },
            messages: vec![Message::user_text("$5 coffee")],
            tools: all_tools(),
            max_tokens: 512,
        };
        let body = build_body("llama3.1:8b-instruct", &req);
        // Ollama: one system message at the front
        assert_eq!(body.messages[0].role, "system");
        assert!(body.messages[0].content.contains("STATIC"));
        assert!(body.messages[0].content.contains("VOLATILE"));
        assert_eq!(body.messages[1].role, "user");
    }

    #[test]
    fn parse_response_with_tool_call() {
        let raw = r#"{
            "model": "llama3.1:8b-instruct",
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    { "function": { "name": "add_expense", "arguments": { "amount": 5, "category": "Coffee" } } }
                ]
            },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 100,
            "eval_count": 20
        }"#;
        let parsed: ApiResponse = serde_json::from_str(raw).unwrap();
        let resp = parsed.into_chat_response();
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        let uses = resp.tool_uses();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].1, "add_expense");
    }
}
