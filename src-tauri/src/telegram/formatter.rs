//! Message formatting helpers for Telegram.
//!
//! By default we send plain text. `escape_md_v2` is here for the day we
//! flip on `parse_mode: MarkdownV2` for richer formatting; it preserves
//! arbitrary user content (expense descriptions, category names) safely.

/// Escape every character Telegram MarkdownV2 treats as syntax. Use this
/// on every dynamic substring before assembling a MarkdownV2 message.
/// Per Telegram docs the full set is `_ * [ ] ( ) ~ \` > # + - = | { } . !`.
pub fn escape_md_v2(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for c in input.chars() {
        match c {
            '_' | '*' | '[' | ']' | '(' | ')' | '~' | '`' | '>' | '#' | '+' | '-' | '=' | '|'
            | '{' | '}' | '.' | '!' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Render an integer cents amount as a string in major units. Defaults to
/// 2 decimal places. Currency is a 3-letter code; we render a generic
/// prefix because emojis/symbols are user-locale dependent.
pub fn format_money(amount_cents: i64, currency: &str) -> String {
    let abs = amount_cents.unsigned_abs();
    let major = abs / 100;
    let minor = abs % 100;
    let sign = if amount_cents < 0 { "-" } else { "" };
    match currency {
        "USD" => format!("{sign}${major}.{minor:02}"),
        "EUR" => format!("{sign}€{major}.{minor:02}"),
        "GBP" => format!("{sign}£{major}.{minor:02}"),
        "JPY" => format!("{sign}¥{}", abs / 100),
        other => format!("{sign}{major}.{minor:02} {other}"),
    }
}

/// Standard `/help` reply.
pub fn help_text() -> String {
    "Hi, I'm Mr. Moneypenny.\n\n\
     Tell me about an expense in plain English:\n\
     • $5 coffee\n\
     • paid rent 1500\n\
     • $22.50 dining at Pho 88\n\n\
     Or ask me a question:\n\
     • how am I doing this month\n\
     • how much have I spent on coffee this week\n\
     • how much did Spouse spend on dining\n\n\
     Commands:\n\
     /start <code> – pair this chat with the desktop app\n\
     /help – this message\n\
     /undo – delete the last expense you logged\n\
     /cancel – cancel a pending confirmation\n\n\
     Your data lives only on the host computer. I never copy it anywhere."
        .to_string()
}

/// Polite reply for chats that are not on the whitelist.
pub fn unauthorized_text() -> String {
    "Hi — this is a private bot. Pair this chat with your desktop app to use it.\n\
     Get a 6-digit code from Settings → Household → Invite member, \
     then send /start <code> here."
        .to_string()
}

/// Reply when a `/start <code>` succeeds.
pub fn paired_text(display_name: &str, role: &str) -> String {
    format!(
        "Welcome, {display_name}. You're paired as {role}.\n\
         Try logging an expense, e.g. \"$5 coffee\"."
    )
}

/// Reply when a `/start <code>` fails.
pub fn pairing_failed_text(reason: &str) -> String {
    format!("Couldn't pair this chat: {reason}.\n\nGenerate a fresh code in the desktop app and try again.")
}

/// Standard reply for `/cancel` when nothing is pending.
pub fn nothing_to_cancel_text() -> String {
    "Nothing pending to cancel.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_handles_all_special_chars() {
        let raw = "Hello _world_ *bold* (parens) [brackets] ~strike~ `code` >quote #1+2-3=4|5{6}7.8!9\\back";
        let escaped = escape_md_v2(raw);
        // Every special char should have a leading backslash; alphabetics untouched.
        for c in [
            '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.',
            '!', '\\',
        ] {
            assert!(
                !escaped.contains(c) || escaped.contains(&format!("\\{c}")),
                "{c} not properly escaped in {escaped}"
            );
        }
        assert!(escaped.contains("Hello"));
    }

    #[test]
    fn money_format_usd() {
        assert_eq!(format_money(0, "USD"), "$0.00");
        assert_eq!(format_money(500, "USD"), "$5.00");
        assert_eq!(format_money(799, "USD"), "$7.99");
        assert_eq!(format_money(150_000, "USD"), "$1500.00");
        assert_eq!(format_money(-300, "USD"), "-$3.00");
    }

    #[test]
    fn money_format_eur_and_gbp() {
        assert_eq!(format_money(2_550, "EUR"), "€25.50");
        assert_eq!(format_money(99, "GBP"), "£0.99");
    }

    #[test]
    fn money_format_jpy_no_decimals() {
        assert_eq!(format_money(10_000, "JPY"), "¥100");
    }

    #[test]
    fn money_format_unknown_currency_uses_code_suffix() {
        assert_eq!(format_money(1_234, "ZAR"), "12.34 ZAR");
    }

    #[test]
    fn paired_text_includes_role() {
        let s = paired_text("Wyatt", "owner");
        assert!(s.contains("Wyatt"));
        assert!(s.contains("owner"));
    }
}
