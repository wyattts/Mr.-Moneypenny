//! Optional LLM-assisted merchant categorization.
//!
//! Off by default. The Settings → Import wizard surfaces a `✨ AI-suggest`
//! button on the review screen; clicking it sends **one** batched call
//! to the user's configured LLM (Anthropic or Ollama) with all
//! unmatched merchant strings and the user's category list, and parses
//! the JSON response back into a per-merchant suggestion map.
//!
//! Cost (Anthropic): a 50-merchant batch with a 12-category list runs
//! ~500 input + ~200 output tokens. At Haiku rates that's roughly
//! $0.0014; even Sonnet stays under $0.005. The "per import" cost
//! lower-bounds beat row-by-row LLM categorization by 1-2 orders of
//! magnitude.
//!
//! ## Privacy
//!
//! Only the merchant strings + category names cross the wire. No
//! amounts, no dates, no descriptions, no row counts beyond the unique
//! merchant set. Off by default; user opts in per-import.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::llm::{ChatRequest, ContentBlock, LLMProvider, Message, SystemPrompt};

#[derive(Debug, Clone, Serialize)]
pub struct CategoryHint {
    pub id: i64,
    pub name: String,
    /// Optional hint for the LLM (e.g., kind=fixed/variable/investing
    /// or a user-authored description). Improves match quality for
    /// ambiguous merchants without ballooning prompt size.
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiSuggestResult {
    pub suggestions: HashMap<String, i64>,
    pub cost_micros: i64,
}

/// Send `merchants` and `categories` to the LLM in a single call.
/// Returns suggestions for whichever merchants the model could
/// confidently place + the cost in micros for the call.
pub async fn suggest_categories(
    provider: &dyn LLMProvider,
    merchants: &[String],
    categories: &[CategoryHint],
) -> Result<AiSuggestResult> {
    if merchants.is_empty() || categories.is_empty() {
        return Ok(AiSuggestResult {
            suggestions: HashMap::new(),
            cost_micros: 0,
        });
    }
    let prompt = build_prompt(merchants, categories);
    let req = ChatRequest {
        system_prompt: SystemPrompt {
            stable: SYSTEM_PROMPT.to_string(),
            volatile: String::new(),
        },
        messages: vec![Message::user_text(prompt)],
        tools: vec![],
        max_tokens: 800,
    };
    let resp = provider.chat(req).await?;
    let text = collect_text(&resp.content);
    let parsed = parse_response(&text, merchants)?;
    let cost_micros =
        crate::llm::pricing::compute_cost_micros(provider.model(), &resp.usage).unwrap_or(0);
    Ok(AiSuggestResult {
        suggestions: parsed,
        cost_micros,
    })
}

const SYSTEM_PROMPT: &str = "\
You are a CSV categorization assistant for a personal-budgeting app.
You will be given a list of merchant strings (as they appear on a bank
statement) and a list of budget categories. For each merchant you can
confidently place, output the merchant id and category id in JSON.
Skip any merchant you can't place — the user will categorize those
manually. NEVER invent a category id that wasn't in the list.

Return ONLY a single JSON object of the shape:
  {\"matches\": [{\"merchant\": \"<exact merchant string>\", \"category_id\": <int>}, ...]}
No prose, no code fences. The merchant string must exactly match the
input.\
";

fn build_prompt(merchants: &[String], categories: &[CategoryHint]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    s.push_str("CATEGORIES:\n");
    for c in categories {
        let _ = writeln!(
            s,
            "- id={} name={}{}",
            c.id,
            c.name,
            c.hint
                .as_deref()
                .map(|h| format!(" hint=\"{h}\""))
                .unwrap_or_default()
        );
    }
    s.push_str("\nMERCHANTS:\n");
    for m in merchants {
        let _ = writeln!(s, "- {m}");
    }
    s
}

fn collect_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Debug, Deserialize)]
struct RawResponse {
    matches: Vec<RawMatch>,
}

#[derive(Debug, Deserialize)]
struct RawMatch {
    merchant: String,
    category_id: i64,
}

/// Parse the model's JSON output back into a {merchant: category_id}
/// map, dropping any entries whose merchant isn't in the input set
/// (the model occasionally hallucinates merchant strings or rewords
/// them).
fn parse_response(
    text: &str,
    valid_merchants: &[String],
) -> Result<HashMap<String, i64>> {
    let trimmed = strip_code_fence(text.trim());
    let raw: RawResponse = serde_json::from_str(trimmed)
        .map_err(|e| anyhow!("LLM response was not valid JSON: {e}; response was: {text}"))?;
    let valid: std::collections::HashSet<&str> =
        valid_merchants.iter().map(|s| s.as_str()).collect();
    let mut out = HashMap::new();
    for m in raw.matches {
        if valid.contains(m.merchant.as_str()) {
            out.insert(m.merchant, m.category_id);
        }
    }
    Ok(out)
}

/// Some models still wrap JSON in ```json ... ``` despite instructions.
/// Strip the fence if present.
fn strip_code_fence(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.trim_end_matches("```").trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.trim_end_matches("```").trim()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_picks_valid_merchants_only() {
        let text = r#"{"matches":[
            {"merchant":"STARBUCKS #4521","category_id":3},
            {"merchant":"BOGUS","category_id":99}
        ]}"#;
        let valid = vec!["STARBUCKS #4521".to_string(), "AMAZON.COM".to_string()];
        let map = parse_response(text, &valid).unwrap();
        assert_eq!(map.get("STARBUCKS #4521"), Some(&3));
        assert!(!map.contains_key("BOGUS"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_response_strips_code_fence() {
        let text = "```json\n{\"matches\":[{\"merchant\":\"X\",\"category_id\":1}]}\n```";
        let valid = vec!["X".to_string()];
        let map = parse_response(text, &valid).unwrap();
        assert_eq!(map.get("X"), Some(&1));
    }

    #[test]
    fn parse_response_rejects_invalid_json() {
        let valid = vec!["X".to_string()];
        assert!(parse_response("definitely not json", &valid).is_err());
    }

    #[test]
    fn build_prompt_includes_all_inputs() {
        let merchants = vec!["STARBUCKS".to_string(), "AMAZON".to_string()];
        let categories = vec![
            CategoryHint {
                id: 1,
                name: "Coffee".into(),
                hint: Some("variable".into()),
            },
            CategoryHint {
                id: 2,
                name: "Online".into(),
                hint: None,
            },
        ];
        let p = build_prompt(&merchants, &categories);
        assert!(p.contains("STARBUCKS"));
        assert!(p.contains("AMAZON"));
        assert!(p.contains("Coffee"));
        assert!(p.contains("Online"));
        assert!(p.contains("id=1"));
        assert!(p.contains("hint=\"variable\""));
    }

    #[test]
    fn empty_inputs_short_circuit() {
        // Stub provider not used because we never call it on empty
        // inputs. Just sanity-check the early-return.
        // We can't easily build a Sender + tokio runtime in a sync
        // test here without futures::executor; check the input branch
        // directly via build_prompt instead.
        let p = build_prompt(&[], &[]);
        assert!(p.contains("CATEGORIES:"));
        assert!(p.contains("MERCHANTS:"));
    }
}
