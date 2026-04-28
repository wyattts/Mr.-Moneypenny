//! In-memory bot state: per-chat conversation history.
//!
//! History is kept bounded (last `MAX_TURNS` messages, dropping anything
//! older than `HISTORY_TTL`). It survives in process memory only — if
//! the desktop is restarted, history starts fresh, which is fine for
//! a budgeting bot.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use time::{Duration, OffsetDateTime};

use crate::llm::Message;

/// Hard cap per chat. Keeps prompts cheap and avoids unbounded growth.
const MAX_TURNS: usize = 24;
/// Anything older than this is dropped. The LLM gets the database via
/// tools; long-term memory lives in SQLite, not in chat history.
const HISTORY_TTL: Duration = Duration::minutes(30);

#[derive(Debug, Clone)]
struct TimedMessage {
    message: Message,
    at: OffsetDateTime,
}

#[derive(Debug, Default)]
pub struct ConversationStore {
    chats: HashMap<i64, VecDeque<TimedMessage>>,
}

impl ConversationStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a message and prune.
    pub fn append(&mut self, chat_id: i64, message: Message, now: OffsetDateTime) {
        let entry = self.chats.entry(chat_id).or_default();
        entry.push_back(TimedMessage { message, at: now });
        Self::prune(entry, now);
    }

    /// Return a fresh copy of the history (after pruning).
    pub fn history(&mut self, chat_id: i64, now: OffsetDateTime) -> Vec<Message> {
        let entry = self.chats.entry(chat_id).or_default();
        Self::prune(entry, now);
        entry.iter().map(|t| t.message.clone()).collect()
    }

    /// Drop all stored history for a chat. Used on `/cancel` or unpair.
    pub fn clear(&mut self, chat_id: i64) {
        self.chats.remove(&chat_id);
    }

    fn prune(entry: &mut VecDeque<TimedMessage>, now: OffsetDateTime) {
        let cutoff = now - HISTORY_TTL;
        while entry.front().map(|t| t.at < cutoff).unwrap_or(false) {
            entry.pop_front();
        }
        while entry.len() > MAX_TURNS {
            entry.pop_front();
        }
    }
}

/// Shareable bot state container.
pub struct BotState {
    pub conversations: Mutex<ConversationStore>,
}

impl BotState {
    pub fn new() -> Self {
        Self {
            conversations: Mutex::new(ConversationStore::new()),
        }
    }
}

impl Default for BotState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn prunes_old_messages() {
        let mut store = ConversationStore::new();
        let chat = 1;
        let t0 = datetime!(2026-04-28 12:00:00 UTC);
        store.append(chat, Message::user_text("old"), t0);

        // 31 minutes later — older message should be dropped
        let later = t0 + Duration::minutes(31);
        store.append(chat, Message::user_text("new"), later);
        let h = store.history(chat, later);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn enforces_max_turns() {
        let mut store = ConversationStore::new();
        let chat = 1;
        let now = datetime!(2026-04-28 12:00:00 UTC);
        for i in 0..30 {
            store.append(chat, Message::user_text(format!("msg {i}")), now);
        }
        let h = store.history(chat, now);
        assert_eq!(h.len(), MAX_TURNS);
    }

    #[test]
    fn clear_drops_chat_history() {
        let mut store = ConversationStore::new();
        let chat = 1;
        let now = datetime!(2026-04-28 12:00:00 UTC);
        store.append(chat, Message::user_text("hello"), now);
        store.clear(chat);
        assert!(store.history(chat, now).is_empty());
    }
}
