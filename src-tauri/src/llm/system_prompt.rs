//! System-prompt assembly.
//!
//! The prompt is split into a **stable** portion (rarely changes — eligible
//! for Anthropic prompt caching) and a **volatile** portion (today's date,
//! current category list, authorized-chat context — re-built each turn).

use std::fmt::Write;

use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::domain::CategoryKind;

#[derive(Debug, Clone)]
pub struct SystemPrompt {
    /// Doesn't change between requests. Provider may apply prompt caching.
    pub stable: String,
    /// Per-request: date, categories, authorized-chat context.
    pub volatile: String,
}

#[derive(Debug, Clone)]
pub struct SystemPromptInput {
    pub now: OffsetDateTime,
    /// Display name of the chat that sent the message we're answering.
    pub authorized_chat_name: Option<String>,
    /// "owner" or "member".
    pub authorized_chat_role: Option<String>,
    /// `(name, kind)` pairs for active categories. Slim list — full details
    /// available via `list_categories` tool if the LLM needs them.
    pub categories: Vec<(String, CategoryKind)>,
    pub user_currency: String,
    /// Names of all household members. Only included when more than one.
    pub household_members: Vec<String>,
}

const STABLE: &str = r#"You are Mr. Moneypenny, a polite, butler-toned personal-budgeting assistant. You help one user (or one household) log expenses, query their spending, and stay on top of their budget.

# Operating principles

1. **Use tools, never invent numbers.** Every claim about money must come from a tool call result. If you don't know something, call a tool. The available tools are listed below.
2. **Tool-use only — never SQL.** You will never see or generate SQL. All database access is via the typed tools.
3. **Distinguish fixed from variable.** Fixed categories (rent, insurance, subscriptions) are inevitable. Variable categories (groceries, dining, coffee) are discretionary. When the user asks "how am I doing this month", call `summarize_period` and pace the user against their VARIABLE budget. Do NOT say things like "you're terrible" because rent posted — rent was always going to be paid.
4. **Confirm before destructive actions.** Before calling `delete_expense` or `set_budget`, confirm with the user in plain language and wait for their "yes" / "confirm" reply. Never assume.
5. **Be concise.** Telegram messages are read on phones. Keep replies short. Use bullet points and bold for emphasis sparingly. Numbers and short sentences beat paragraphs.
6. **Be honest about uncertainty.** If a category isn't in the user's list, say so and offer to add it (don't silently coerce). If the LLM-confidence on a parse is low, ask before logging.
7. **Currency formatting.** Use the symbol and decimals appropriate to the user's locale. Always include cents for amounts under $1000. For larger amounts, you may round to the nearest dollar in summaries.

# How users typically talk to you

- Logging: "$5 coffee", "spent 47 on groceries", "paid rent 1500", "$22.50 dining at Pho 88"
- Querying: "how am I doing this month", "how much did I spend on coffee this week", "what's left in my dining budget"
- Editing: "delete that last one", "actually that was Groceries not Dining"

# Tool selection cheatsheet

- User describes a new expense → `add_expense`
- User asks for a budget summary or "how am I doing" → `summarize_period`
- User asks for a specific spend total → `query_expenses`
- User asks "what categories do I have" or you don't know an exact category name → `list_categories`
- User wants to change a budget → confirm, then `set_budget`
- User asks about a household member by name → `list_household_members` then `query_expenses` filtered by them
- User confirms deletion → `delete_expense`

# Output style

- Cheerful but not cloying. Brief. Like a competent butler.
- After logging, briefly confirm: "Logged $5 for Coffee."
- After summarizing, lead with the headline ("On pace this month — $42 a day to spend.") then optional context.
"#;

pub fn build_system_prompt(input: &SystemPromptInput) -> SystemPrompt {
    SystemPrompt {
        stable: STABLE.to_string(),
        volatile: build_volatile(input),
    }
}

fn build_volatile(input: &SystemPromptInput) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Current context");

    let date_str = input
        .now
        .format(&Iso8601::DEFAULT)
        .unwrap_or_else(|_| input.now.to_string());
    let _ = writeln!(out, "- Today (user's local time): {date_str}");
    let _ = writeln!(out, "- User's currency: {}", input.user_currency);

    if let Some(name) = &input.authorized_chat_name {
        let role = input.authorized_chat_role.as_deref().unwrap_or("member");
        let _ = writeln!(
            out,
            "- You are talking to: {name} ({role}). Attribute new expenses to them automatically."
        );
    }

    if input.household_members.len() > 1 {
        let _ = writeln!(
            out,
            "- Household members: {}",
            input.household_members.join(", ")
        );
    }

    let _ = writeln!(out, "\n# Active categories\n");
    let mut fixed: Vec<&str> = Vec::new();
    let mut variable: Vec<&str> = Vec::new();
    for (name, kind) in &input.categories {
        match kind {
            CategoryKind::Fixed => fixed.push(name.as_str()),
            CategoryKind::Variable => variable.push(name.as_str()),
        }
    }
    if !fixed.is_empty() {
        let _ = writeln!(out, "Fixed: {}", fixed.join(", "));
    }
    if !variable.is_empty() {
        let _ = writeln!(out, "Variable: {}", variable.join(", "));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn volatile_includes_date_and_chat() {
        let p = build_system_prompt(&SystemPromptInput {
            now: datetime!(2026-04-28 12:00:00 UTC),
            authorized_chat_name: Some("Wyatt".into()),
            authorized_chat_role: Some("owner".into()),
            categories: vec![
                ("Coffee".into(), CategoryKind::Variable),
                ("Rent / Mortgage".into(), CategoryKind::Fixed),
            ],
            user_currency: "USD".into(),
            household_members: vec!["Wyatt".into(), "Spouse".into()],
        });
        assert!(p.volatile.contains("Wyatt (owner)"));
        assert!(p.volatile.contains("2026-04-28"));
        assert!(p.volatile.contains("Fixed: Rent / Mortgage"));
        assert!(p.volatile.contains("Variable: Coffee"));
        assert!(p.volatile.contains("Household members: Wyatt, Spouse"));
    }

    #[test]
    fn volatile_omits_household_when_solo() {
        let p = build_system_prompt(&SystemPromptInput {
            now: datetime!(2026-04-28 12:00:00 UTC),
            authorized_chat_name: Some("Wyatt".into()),
            authorized_chat_role: Some("owner".into()),
            categories: vec![],
            user_currency: "USD".into(),
            household_members: vec!["Wyatt".into()],
        });
        assert!(!p.volatile.contains("Household members:"));
    }

    #[test]
    fn stable_is_self_contained() {
        let p = build_system_prompt(&SystemPromptInput {
            now: datetime!(2026-04-28 12:00:00 UTC),
            authorized_chat_name: None,
            authorized_chat_role: None,
            categories: vec![],
            user_currency: "USD".into(),
            household_members: vec![],
        });
        assert!(p.stable.contains("Mr. Moneypenny"));
        assert!(p.stable.contains("Use tools"));
        assert!(p.stable.contains("fixed from variable"));
    }
}
