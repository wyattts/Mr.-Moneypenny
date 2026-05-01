//! Descriptive statistics over a series of i64 cents.
//!
//! Powers the per-category "what does my spending look like" panel in
//! the v0.3.0 Forecast view. Returns `None` when N is too small to be
//! meaningful — better to show "not enough data yet (N=2)" than to
//! report a "median" computed over two points.
//!
//! Histograms use simple equal-width bucketing; the caller picks the
//! number of buckets (default ~12 for monthly views).

use serde::Serialize;

/// Minimum sample size below which descriptive stats refuse to compute.
/// Set low (≥3) so the user gets *something* even with sparse data;
/// the dashboard surfaces the count so users can judge for themselves.
pub const MIN_N: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct DescriptiveStats {
    pub n: usize,
    pub min_cents: i64,
    pub max_cents: i64,
    pub mean_cents: i64,
    pub median_cents: i64,
    pub p10_cents: i64,
    pub p90_cents: i64,
    pub stddev_cents: i64,
}

/// Compute descriptive stats over a slice of cents amounts. Returns
/// `None` when `values.len() < MIN_N`.
pub fn describe(values: &[i64]) -> Option<DescriptiveStats> {
    let n = values.len();
    if n < MIN_N {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();

    let min_cents = *sorted.first().unwrap();
    let max_cents = *sorted.last().unwrap();
    let sum: i128 = sorted.iter().map(|&v| v as i128).sum();
    let mean_cents = (sum / n as i128) as i64;
    let median_cents = percentile(&sorted, 50.0);
    let p10_cents = percentile(&sorted, 10.0);
    let p90_cents = percentile(&sorted, 90.0);

    // Population std-dev (we have the full sample, not an inferred parent
    // distribution, so n is the right divisor here).
    let mean = mean_cents as f64;
    let variance = sorted
        .iter()
        .map(|&v| {
            let d = v as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / n as f64;
    let stddev_cents = variance.sqrt().round() as i64;

    Some(DescriptiveStats {
        n,
        min_cents,
        max_cents,
        mean_cents,
        median_cents,
        p10_cents,
        p90_cents,
        stddev_cents,
    })
}

/// Linear-interpolated percentile over a pre-sorted slice. `pct` is
/// 0..=100. Returns the i64 floor of the interpolated value.
pub fn percentile(sorted: &[i64], pct: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let p = pct.clamp(0.0, 100.0) / 100.0;
    let idx = p * (sorted.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = idx - lo as f64;
        let v = sorted[lo] as f64 + (sorted[hi] - sorted[lo]) as f64 * frac;
        v.round() as i64
    }
}

/// Equal-width histogram bucketing. Returns a vector of bucket counts
/// where bucket `i` covers `[min + i*width, min + (i+1)*width)` (the
/// last bucket is right-inclusive).
#[derive(Debug, Clone, Serialize)]
pub struct Histogram {
    pub bucket_count: usize,
    pub min_cents: i64,
    pub max_cents: i64,
    pub bucket_width_cents: i64,
    pub counts: Vec<usize>,
}

pub fn histogram(values: &[i64], buckets: usize) -> Option<Histogram> {
    if values.is_empty() || buckets == 0 {
        return None;
    }
    let min_cents = *values.iter().min().unwrap();
    let max_cents = *values.iter().max().unwrap();
    if min_cents == max_cents {
        // Degenerate: all values identical. Single bucket holding all.
        return Some(Histogram {
            bucket_count: 1,
            min_cents,
            max_cents,
            bucket_width_cents: 0,
            counts: vec![values.len()],
        });
    }
    let span = (max_cents - min_cents) as f64;
    let width = (span / buckets as f64).ceil() as i64;
    // After ceiling, recompute actual bucket_count (might be slightly less).
    let mut counts = vec![0usize; buckets];
    for &v in values {
        let i = ((v - min_cents) / width) as usize;
        let i = i.min(buckets - 1); // last value → last bucket
        counts[i] += 1;
    }
    Some(Histogram {
        bucket_count: buckets,
        min_cents,
        max_cents,
        bucket_width_cents: width,
        counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_slice_returns_none() {
        assert!(describe(&[]).is_none());
    }

    #[test]
    fn below_min_n_returns_none() {
        assert!(describe(&[100, 200]).is_none()); // N=2 < 3
        assert!(describe(&[100, 200, 300]).is_some()); // N=3 = MIN_N
    }

    #[test]
    fn known_inputs_match_known_outputs() {
        // 100, 200, 300, 400, 500
        let s = describe(&[100, 200, 300, 400, 500]).unwrap();
        assert_eq!(s.n, 5);
        assert_eq!(s.min_cents, 100);
        assert_eq!(s.max_cents, 500);
        assert_eq!(s.mean_cents, 300);
        assert_eq!(s.median_cents, 300);
        // Population stddev = sqrt(((-200)^2 + (-100)^2 + 0 + 100^2 + 200^2)/5)
        //                   = sqrt(100000/5) = sqrt(20000) ≈ 141.42
        assert_eq!(s.stddev_cents, 141);
    }

    #[test]
    fn percentile_linearly_interpolates() {
        let sorted = vec![100, 200, 300, 400, 500];
        assert_eq!(percentile(&sorted, 0.0), 100);
        assert_eq!(percentile(&sorted, 50.0), 300);
        assert_eq!(percentile(&sorted, 100.0), 500);
        // 25% → idx 1.0 → exact 200
        assert_eq!(percentile(&sorted, 25.0), 200);
        // 10% → idx 0.4 → 100 + (200-100)*0.4 = 140
        assert_eq!(percentile(&sorted, 10.0), 140);
    }

    #[test]
    fn p10_and_p90_bracket_the_median() {
        let s = describe(&[10, 50, 100, 150, 200, 250, 300, 350, 400, 450]).unwrap();
        assert!(s.p10_cents < s.median_cents);
        assert!(s.median_cents < s.p90_cents);
    }

    #[test]
    fn all_equal_values_give_zero_stddev() {
        let s = describe(&[500, 500, 500, 500]).unwrap();
        assert_eq!(s.stddev_cents, 0);
        assert_eq!(s.mean_cents, 500);
        assert_eq!(s.median_cents, 500);
    }

    #[test]
    fn histogram_distributes_values_into_buckets() {
        let h = histogram(&[100, 100, 200, 300, 400, 500], 5).unwrap();
        assert_eq!(h.counts.len(), 5);
        assert_eq!(h.counts.iter().sum::<usize>(), 6, "all values bucketed");
        // Min == 100, max == 500, span = 400, width = ceil(400/5) = 80.
        // 100 → bucket 0; 200 → bucket 1; 300 → bucket 2; 400 → bucket 3; 500 → bucket 4
        assert_eq!(h.counts[0], 2); // two 100s
        assert_eq!(h.counts[4], 1); // one 500
    }

    #[test]
    fn histogram_handles_all_equal_with_single_bucket() {
        let h = histogram(&[200, 200, 200], 5).unwrap();
        assert_eq!(h.bucket_count, 1);
        assert_eq!(h.counts, vec![3]);
    }

    #[test]
    fn histogram_empty_returns_none() {
        assert!(histogram(&[], 5).is_none());
    }
}
