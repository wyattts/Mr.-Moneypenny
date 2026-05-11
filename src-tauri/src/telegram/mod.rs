// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
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
