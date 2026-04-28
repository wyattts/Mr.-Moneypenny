//! Expense — a single recorded transaction.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExpenseSource {
    /// Logged via the Telegram bot conversation.
    Telegram,
    /// Logged manually through the desktop GUI.
    Manual,
}

impl ExpenseSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExpenseSource::Telegram => "telegram",
            ExpenseSource::Manual => "manual",
        }
    }
}

impl std::str::FromStr for ExpenseSource {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "telegram" => Ok(ExpenseSource::Telegram),
            "manual" => Ok(ExpenseSource::Manual),
            other => anyhow::bail!("invalid expense source: {other}"),
        }
    }
}

impl ToSql for ExpenseSource {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for ExpenseSource {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: anyhow::Error| FromSqlError::Other(e.into()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expense {
    pub id: i64,
    pub amount_cents: i64,
    pub currency: String,
    pub category_id: Option<i64>,
    pub description: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub source: ExpenseSource,
    pub raw_message: Option<String>,
    pub llm_confidence: Option<f64>,
    pub logged_by_chat_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewExpense {
    pub amount_cents: i64,
    pub currency: String,
    pub category_id: Option<i64>,
    pub description: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    pub source: ExpenseSource,
    pub raw_message: Option<String>,
    pub llm_confidence: Option<f64>,
    pub logged_by_chat_id: Option<i64>,
}
