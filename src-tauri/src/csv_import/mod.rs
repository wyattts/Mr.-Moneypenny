//! CSV import pipeline. v0.3.2.
//!
//! Bulk-imports bank/credit-card CSV exports into the local expense
//! ledger without round-tripping every row through the LLM.
//!
//! The pipeline has four stages:
//!
//! 1. **Parse** (`parser`) — read the file, apply the user's column
//!    mapping, produce typed `ParsedRow`s.
//! 2. **Categorize** (`categorize`) — three-layer match: saved
//!    `merchant_rules`, fuzzy match against existing expense
//!    descriptions, then anything left flows to a manual review queue
//!    in the UI. Optional batched LLM call (`ai_suggest`) helps
//!    populate the queue.
//! 3. **Dedupe** (`dedupe`) — within-CSV + against-DB Levenshtein
//!    pass. Flagged rows are surfaced in a review screen, default-skip.
//! 4. **Commit** — caller-driven; writes accepted rows into the
//!    `expenses` table with `source = 'csv'`.
//!
//! See `Patches/v0.3.2.md` for the full design spec.

pub mod ai_suggest;
pub mod categorize;
pub mod dedupe;
pub mod parser;

pub use parser::{parse_preview, parse_with_mapping, ParsedRow, PreviewResult};
