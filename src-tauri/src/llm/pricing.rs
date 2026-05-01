//! Hardcoded model price table for cost computation.
//!
//! Anthropic publishes prices in USD per million tokens. This module
//! mirrors them and exposes `compute_cost_micros` which the call-site
//! invokes after each successful response, before persisting a row to
//! `llm_usage`.
//!
//! ## Maintenance
//!
//! Update this file when:
//!   * Anthropic adjusts pricing for an existing model.
//!   * A new Claude model ships and we expose it in Settings.
//!
//! Prices below reflect the public Anthropic rates in late 2025 / early
//! 2026. Cache pricing follows the standard multipliers:
//!   * Cache READ:  0.1 × input price
//!   * Cache CREATE (5-min): 1.25 × input price (we don't use the 1h flavor)
//!
//! Unknown models — including any Ollama model — return `None` and the
//! call-site logs them with `cost_micros = 0` (still useful for call
//! counts).

use super::Usage;

#[derive(Debug, Clone, Copy)]
pub struct ModelPrice {
    /// USD per million input tokens.
    pub input_per_mtok_usd: f64,
    pub output_per_mtok_usd: f64,
    pub cache_read_per_mtok_usd: f64,
    pub cache_creation_per_mtok_usd: f64,
}

impl ModelPrice {
    /// Construct a price from the input + output rates, deriving the
    /// cache-read and cache-creation rates with the standard
    /// multipliers (0.1× and 1.25×).
    const fn from_input_output(input: f64, output: f64) -> Self {
        Self {
            input_per_mtok_usd: input,
            output_per_mtok_usd: output,
            cache_read_per_mtok_usd: input * 0.1,
            cache_creation_per_mtok_usd: input * 1.25,
        }
    }
}

/// Look up a model's pricing. Matches dated IDs (e.g.
/// `claude-haiku-4-5-20251001`) by prefix so we don't have to add a row
/// for every snapshot.
pub fn pricing(model: &str) -> Option<ModelPrice> {
    if model.starts_with("claude-haiku-4-7") || model.starts_with("claude-haiku-4-6") {
        // Haiku 4.6/4.7 launch pricing.
        return Some(ModelPrice::from_input_output(1.0, 5.0));
    }
    if model.starts_with("claude-haiku-4-5") {
        return Some(ModelPrice::from_input_output(1.0, 5.0));
    }
    if model.starts_with("claude-sonnet-4-7")
        || model.starts_with("claude-sonnet-4-6")
        || model.starts_with("claude-sonnet-4-5")
    {
        return Some(ModelPrice::from_input_output(3.0, 15.0));
    }
    if model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-6")
        || model.starts_with("claude-opus-4-5")
    {
        return Some(ModelPrice::from_input_output(15.0, 75.0));
    }
    None
}

/// Compute cost in micro-dollars given a model + token counts.
/// Returns `None` for unknown / unpriced models.
pub fn compute_cost_micros(model: &str, usage: &Usage) -> Option<i64> {
    let p = pricing(model)?;
    let dollars = p.input_per_mtok_usd * (usage.input_tokens as f64) / 1_000_000.0
        + p.output_per_mtok_usd * (usage.output_tokens as f64) / 1_000_000.0
        + p.cache_read_per_mtok_usd * (usage.cache_read_input_tokens as f64) / 1_000_000.0
        + p.cache_creation_per_mtok_usd * (usage.cache_creation_input_tokens as f64) / 1_000_000.0;
    Some((dollars * 1_000_000.0).round() as i64)
}

/// Format a micro-dollar amount as USD with the appropriate precision.
/// Tiny amounts get 4 decimal places, normal amounts 2.
pub fn format_micros_usd(micros: i64) -> String {
    let dollars = micros as f64 / 1_000_000.0;
    let abs = dollars.abs();
    if abs >= 1.0 {
        format!("${dollars:.2}")
    } else if abs >= 0.01 {
        format!("${dollars:.3}")
    } else {
        format!("${dollars:.4}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haiku_pricing_matches_expectations() {
        let p = pricing("claude-haiku-4-5-20251001").unwrap();
        assert_eq!(p.input_per_mtok_usd, 1.0);
        assert_eq!(p.output_per_mtok_usd, 5.0);
        assert!((p.cache_read_per_mtok_usd - 0.1).abs() < 1e-9);
        assert!((p.cache_creation_per_mtok_usd - 1.25).abs() < 1e-9);
    }

    #[test]
    fn sonnet_and_opus_have_distinct_prices() {
        let s = pricing("claude-sonnet-4-5").unwrap();
        let o = pricing("claude-opus-4-5").unwrap();
        assert_eq!(s.input_per_mtok_usd, 3.0);
        assert_eq!(o.input_per_mtok_usd, 15.0);
    }

    #[test]
    fn unknown_model_has_no_pricing() {
        assert!(pricing("llama3:8b").is_none());
        assert!(pricing("gpt-5").is_none());
        assert!(pricing("").is_none());
    }

    #[test]
    fn dated_snapshot_ids_match_by_prefix() {
        assert!(pricing("claude-haiku-4-5-20260101").is_some());
        assert!(pricing("claude-sonnet-4-6-20260801").is_some());
    }

    #[test]
    fn compute_cost_haiku_typical_call() {
        // 5K input + 500 output on Haiku:
        //   5000 * 1 / 1M = 0.005 USD
        //    500 * 5 / 1M = 0.0025 USD
        //   total = 0.0075 USD = 7,500 micros
        let u = Usage {
            input_tokens: 5_000,
            output_tokens: 500,
            ..Default::default()
        };
        let cost = compute_cost_micros("claude-haiku-4-5-20251001", &u).unwrap();
        assert_eq!(cost, 7_500);
    }

    #[test]
    fn compute_cost_with_cache_components() {
        // Haiku, 1000 cached read + 200 cache create + 2000 fresh input + 100 output:
        //   cache_read: 1000 * 0.1 / 1M = 0.0001 USD = 100 micros
        //   cache_create: 200 * 1.25 / 1M = 0.00025 USD = 250 micros
        //   input: 2000 * 1 / 1M = 0.002 USD = 2000 micros
        //   output: 100 * 5 / 1M = 0.0005 USD = 500 micros
        //   total = 2,850 micros
        let u = Usage {
            input_tokens: 2_000,
            output_tokens: 100,
            cache_read_input_tokens: 1_000,
            cache_creation_input_tokens: 200,
        };
        let cost = compute_cost_micros("claude-haiku-4-5", &u).unwrap();
        assert_eq!(cost, 2_850);
    }

    #[test]
    fn unknown_model_returns_none_cost() {
        let u = Usage {
            input_tokens: 1000,
            ..Default::default()
        };
        assert!(compute_cost_micros("ollama:llama3", &u).is_none());
    }

    #[test]
    fn format_micros_usd_buckets_precision_by_magnitude() {
        assert_eq!(format_micros_usd(12_340_000), "$12.34");
        assert_eq!(format_micros_usd(50_000), "$0.050");
        assert_eq!(format_micros_usd(7_500), "$0.0075");
        assert_eq!(format_micros_usd(0), "$0.0000");
    }
}
