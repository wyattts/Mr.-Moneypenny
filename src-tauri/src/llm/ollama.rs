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

/// Validate an Ollama endpoint URL before any code hands it to reqwest.
/// Catches audit S-4 — reqwest in Rust does not honor the webview's
/// `connect-src` CSP, so any URL the user (or a frontend bug) writes
/// into `ollama_endpoint` would become a real outbound HTTP target.
///
/// Rules:
///   - Parses cleanly via `url::Url`.
///   - Scheme ∈ {`http`, `https`}.
///   - Length ≤ 2048.
///   - Host must be loopback / RFC1918 / link-local / ULA unless the
///     caller passes `allow_remote = true` (gated on the
///     `ollama_allow_remote` setting in the UI).
///
/// Returns the canonicalized URL string on success.
pub fn validate_endpoint(raw: &str, allow_remote: bool) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(anyhow!("Ollama endpoint is required"));
    }
    if raw.len() > 2048 {
        return Err(anyhow!("Ollama endpoint is too long (max 2048 chars)"));
    }
    let parsed = url::Url::parse(raw).map_err(|e| anyhow!("not a valid URL: {e}"))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(anyhow!(
            "Ollama endpoint scheme must be http or https (got {scheme})"
        ));
    }
    if !allow_remote && !is_local_host(&parsed) {
        return Err(anyhow!(
            "Ollama endpoint host '{}' is not local. To use a remote endpoint, enable \
             \"Allow remote Ollama endpoint\" in Settings.",
            parsed.host_str().unwrap_or("?"),
        ));
    }
    Ok(parsed.to_string())
}

fn is_local_host(u: &url::Url) -> bool {
    // `url::Url::host()` parses the host into typed form, which strips
    // the surrounding brackets that `host_str()` keeps on IPv6 literals.
    match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(v4)) => {
            let ip = std::net::IpAddr::V4(v4);
            ip.is_loopback() || is_private_ip(&ip)
        }
        Some(url::Host::Ipv6(v6)) => {
            let ip = std::net::IpAddr::V6(v6);
            ip.is_loopback() || is_private_ip(&ip)
        }
        None => false,
    }
}

fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            let octets = v6.octets();
            // RFC 4193 ULA (fc00::/7) + RFC 4291 link-local (fe80::/10).
            (octets[0] & 0xfe) == 0xfc || (octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80)
        }
    }
}

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

    fn provider_name(&self) -> &str {
        "ollama"
    }

    fn model(&self) -> &str {
        &self.model
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

    // --- validate_endpoint (audit S-4) ---

    #[test]
    fn validate_accepts_default_localhost() {
        let out = validate_endpoint("http://localhost:11434", false).unwrap();
        assert!(out.starts_with("http://localhost:11434"));
    }

    #[test]
    fn validate_accepts_127_0_0_1_and_ipv6_loopback() {
        validate_endpoint("http://127.0.0.1:11434", false).unwrap();
        validate_endpoint("http://[::1]:11434", false).unwrap();
    }

    #[test]
    fn validate_accepts_private_ranges() {
        validate_endpoint("http://192.168.1.10:11434", false).unwrap();
        validate_endpoint("http://10.0.0.4:11434", false).unwrap();
        validate_endpoint("http://172.16.5.5:11434", false).unwrap();
    }

    #[test]
    fn validate_rejects_public_host_without_opt_in() {
        let err = validate_endpoint("https://ollama.example.com", false).unwrap_err();
        assert!(err.to_string().contains("not local"));
    }

    #[test]
    fn validate_accepts_public_host_with_opt_in() {
        validate_endpoint("https://ollama.example.com", true).unwrap();
    }

    #[test]
    fn validate_rejects_non_http_scheme() {
        let err = validate_endpoint("file:///etc/passwd", true).unwrap_err();
        assert!(err.to_string().contains("scheme must be http"));
    }

    #[test]
    fn validate_rejects_unparseable() {
        let err = validate_endpoint("not a url", false).unwrap_err();
        assert!(err.to_string().contains("not a valid URL"));
    }

    #[test]
    fn validate_rejects_empty() {
        let err = validate_endpoint("   ", false).unwrap_err();
        assert!(err.to_string().contains("required"));
    }

    #[test]
    fn validate_rejects_overlong() {
        let huge = "http://localhost:11434/".to_string() + &"x".repeat(2050);
        let err = validate_endpoint(&huge, false).unwrap_err();
        assert!(err.to_string().contains("too long"));
    }
}
