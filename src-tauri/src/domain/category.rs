//! Categories — fixed (recurring/inevitable), variable (discretionary),
//! or investing (savings / retirement contributions).

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CategoryKind {
    /// Inevitable monthly costs: rent, insurance, subscriptions.
    /// Pacing logic must not penalize users for these.
    Fixed,
    /// Discretionary spend: groceries, dining, entertainment.
    Variable,
    /// Savings + retirement contributions: Roth IRA, 401k, brokerage.
    /// "Spending" against an investing category is a *good* outflow —
    /// hitting or exceeding the monthly target is the goal, not a
    /// cause for the over-budget warning that fixed/variable use.
    Investing,
}

impl CategoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CategoryKind::Fixed => "fixed",
            CategoryKind::Variable => "variable",
            CategoryKind::Investing => "investing",
        }
    }
}

impl std::str::FromStr for CategoryKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fixed" => Ok(CategoryKind::Fixed),
            "variable" => Ok(CategoryKind::Variable),
            "investing" => Ok(CategoryKind::Investing),
            other => anyhow::bail!("invalid category kind: {other}"),
        }
    }
}

impl ToSql for CategoryKind {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for CategoryKind {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: anyhow::Error| FromSqlError::Other(e.into()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub kind: CategoryKind,
    pub monthly_target_cents: Option<i64>,
    pub is_recurring: bool,
    pub recurrence_day_of_month: Option<u8>,
    pub is_active: bool,
    pub is_seed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCategory {
    pub name: String,
    pub kind: CategoryKind,
    pub monthly_target_cents: Option<i64>,
    pub is_recurring: bool,
    pub recurrence_day_of_month: Option<u8>,
}
