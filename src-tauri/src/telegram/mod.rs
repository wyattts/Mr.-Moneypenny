//! Telegram bot integration: typed API client, long-poll loop, message
//! router with the LLM agentic loop, and chat-pairing authentication.

pub mod auth;
pub mod client;
pub mod formatter;
pub mod poller;
pub mod router;
pub mod state;

pub use client::{Chat, Message, TelegramApi, TelegramClient, Update, User};
pub use state::{BotState, ConversationStore};
