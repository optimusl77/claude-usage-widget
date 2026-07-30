/// Extrapolates when a rate-limit window will fill up, based on how its
/// utilization changed across the samples observed so far in the current
/// window instance.
///
/// Each sample is `(timestamp_unix, utilization, reset_unix)`. Only samples
/// that share the same `reset_unix` as the most recent one are used, since a
/// different `reset_unix` means that sample belongs to a previous window
/// (the window already reset since then) and its trend no longer applies.
///
/// Returns `None` when there isn't enough same-window history, utilization
/// isn't trending upward, or the window would reset before it fills at the
/// current pace (in which case a "full at" estimate isn't a useful thing to
/// show).
pub fn estimate_full_at(samples: &[(i64, f64, i64)]) -> Option<i64> {
    let &(_, _, latest_reset) = samples.last()?;
    let same_window: Vec<&(i64, f64, i64)> =
        samples.iter().filter(|(_, _, reset)| *reset == latest_reset).collect();
    if same_window.len() < 2 {
        return None;
    }

    let &(t0, u0, _) = same_window[0];
    let &(t1, u1, reset) = same_window[same_window.len() - 1];
    if t1 <= t0 {
        return None;
    }

    let rate = (u1 - u0) / (t1 - t0) as f64;
    if rate <= 0.0 {
        return None;
    }

    let remaining = (1.0 - u1).max(0.0);
    let seconds_to_full = (remaining / rate).round() as i64;
    let projected = t1 + seconds_to_full;

    if projected >= reset {
        return None;
    }
    Some(projected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rising_utilization_yields_an_estimate() {
        // 0.1 -> 0.3 over 600s, well within a reset far in the future.
        let samples = [(0, 0.1, 100_000), (600, 0.3, 100_000)];
        let result = estimate_full_at(&samples);
        assert_eq!(result, Some(2700));
    }

    #[test]
    fn flat_utilization_yields_no_estimate() {
        let samples = [(0, 0.5, 100_000), (600, 0.5, 100_000)];
        assert_eq!(estimate_full_at(&samples), None);
    }

    #[test]
    fn falling_utilization_yields_no_estimate() {
        let samples = [(0, 0.5, 100_000), (600, 0.3, 100_000)];
        assert_eq!(estimate_full_at(&samples), None);
    }

    #[test]
    fn single_sample_yields_no_estimate() {
        let samples = [(0, 0.5, 100_000)];
        assert_eq!(estimate_full_at(&samples), None);
    }

    #[test]
    fn empty_samples_yield_no_estimate() {
        let samples: [(i64, f64, i64); 0] = [];
        assert_eq!(estimate_full_at(&samples), None);
    }

    #[test]
    fn projection_past_reset_yields_no_estimate() {
        // Barely rising, would only fill long after the window resets.
        let samples = [(0, 0.01, 3600), (1800, 0.02, 3600)];
        assert_eq!(estimate_full_at(&samples), None);
    }

    #[test]
    fn samples_from_a_previous_window_are_ignored() {
        // The first point belongs to an old window (different reset_unix)
        // and should not distort the rate computed from the current one.
        let samples = [(0, 0.9, 500), (1000, 0.05, 100_000), (1600, 0.1, 100_000)];
        let result = estimate_full_at(&samples);
        assert_eq!(result, Some(12400));
    }
}
