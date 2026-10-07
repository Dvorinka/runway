//! Cron scheduling — port of devpush `workers/tasks/cron.py`.
//!
//! Schedule strings are deliberately simple (devpush parity):
//! `"every N minutes"`, `"every N hours"`, `"*/N * * * *"`, or a bare
//! number of minutes. Full cron syntax is out of scope — the runner
//! executes on an interval, not at wall-clock times.

/// Parse a schedule to minutes between runs. 0 = invalid (job gets
/// disabled by the tick, matching devpush).
pub fn parse_schedule(schedule: &str) -> u64 {
    let s = schedule.trim().to_lowercase();
    if let Some(rest) = s.strip_prefix("every ") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if let Some(n) = parts.first().and_then(|p| p.parse::<u64>().ok()) {
            return if parts.get(1).is_some_and(|u| u.contains("hour")) {
                n * 60
            } else {
                n
            };
        }
        return 0;
    }
    if let Some(rest) = s.strip_prefix("*/") {
        return rest
            .split_whitespace()
            .next()
            .and_then(|p| p.parse::<u64>().ok())
            .unwrap_or(0);
    }
    s.parse::<u64>().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_forms() {
        assert_eq!(parse_schedule("every 15 minutes"), 15);
        assert_eq!(parse_schedule("every 2 hours"), 120);
        assert_eq!(parse_schedule("*/30 * * * *"), 30);
        assert_eq!(parse_schedule("45"), 45);
        assert_eq!(parse_schedule("every 1 hour"), 60);
        assert_eq!(parse_schedule("garbage"), 0);
        assert_eq!(parse_schedule("every x days"), 0);
        assert_eq!(parse_schedule(""), 0);
    }
}
