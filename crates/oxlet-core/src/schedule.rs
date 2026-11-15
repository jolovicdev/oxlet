use crate::model::{Automation, Schedule};
use crate::time::local;

const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;

/// Whether `a` should run at `now`. A daily schedule runs once in its hour; the 20-hour floor keeps
/// it from running twice when the clock or the zone moves.
pub fn is_due(a: &Automation, now: u64, offset: i64) -> bool {
    if !a.enabled {
        return false;
    }
    let since = a.last_run.map(|t| now.saturating_sub(t));
    let at = local(now, offset);
    match a.schedule {
        Schedule::EveryMinutes { minutes } => {
            since.is_none_or(|s| s >= u64::from(minutes.max(1)) * MINUTE)
        }
        Schedule::Hourly => since.is_none_or(|s| s >= HOUR),
        Schedule::Daily { hour } => at.hour == hour && since.is_none_or(|s| s >= 20 * HOUR),
        Schedule::Weekdays { hour } => {
            at.weekday < 5 && at.hour == hour && since.is_none_or(|s| s >= 20 * HOUR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderId;

    fn auto(schedule: Schedule, last_run: Option<u64>) -> Automation {
        Automation {
            id: "a".into(),
            project: "p".into(),
            name: "n".into(),
            prompt: "x".into(),
            provider: ProviderId::Codex,
            model: String::new(),
            schedule,
            enabled: true,
            last_run,
            runs: vec![],
        }
    }

    #[test]
    fn intervals_and_daily_hours() {
        let monday_9 = 4 * 86_400_000 + 9 * HOUR;
        assert!(is_due(
            &auto(Schedule::EveryMinutes { minutes: 15 }, None),
            monday_9,
            0
        ));
        assert!(!is_due(
            &auto(
                Schedule::EveryMinutes { minutes: 15 },
                Some(monday_9 - 5 * MINUTE)
            ),
            monday_9,
            0
        ));
        assert!(is_due(
            &auto(Schedule::Daily { hour: 9 }, Some(monday_9 - 24 * HOUR)),
            monday_9,
            0
        ));
        assert!(!is_due(
            &auto(Schedule::Daily { hour: 9 }, Some(monday_9 - 10 * MINUTE)),
            monday_9,
            0
        ));
        assert!(!is_due(
            &auto(Schedule::Daily { hour: 10 }, None),
            monday_9,
            0
        ));
        let saturday_9 = monday_9 + 5 * 24 * HOUR;
        assert!(!is_due(
            &auto(Schedule::Weekdays { hour: 9 }, None),
            saturday_9,
            0
        ));
        let mut off = auto(Schedule::Hourly, None);
        off.enabled = false;
        assert!(!is_due(&off, monday_9, 0));
    }
}
