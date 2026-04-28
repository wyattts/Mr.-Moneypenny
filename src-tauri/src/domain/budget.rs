//! Budget — a monetary cap on a category over a period.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetPeriod {
    Weekly,
    Monthly,
    Yearly,
}

impl BudgetPeriod {
    pub fn as_str(&self) -> &'static str {
        match self {
            BudgetPeriod::Weekly => "weekly",
            BudgetPeriod::Monthly => "monthly",
            BudgetPeriod::Yearly => "yearly",
        }
    }
}

impl std::str::FromStr for BudgetPeriod {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "weekly" => Ok(BudgetPeriod::Weekly),
            "monthly" => Ok(BudgetPeriod::Monthly),
            "yearly" => Ok(BudgetPeriod::Yearly),
            other => anyhow::bail!("invalid budget period: {other}"),
        }
    }
}

impl ToSql for BudgetPeriod {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for BudgetPeriod {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: anyhow::Error| FromSqlError::Other(e.into()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub id: i64,
    pub category_id: i64,
    pub amount_cents: i64,
    pub period: BudgetPeriod,
    #[serde(with = "time::serde::rfc3339")]
    pub effective_from: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub effective_to: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewBudget {
    pub category_id: i64,
    pub amount_cents: i64,
    pub period: BudgetPeriod,
    #[serde(with = "time::serde::rfc3339")]
    pub effective_from: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub effective_to: Option<OffsetDateTime>,
}
