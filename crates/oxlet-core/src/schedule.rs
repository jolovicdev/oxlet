use crate::model::{Automation, Schedule};
use crate::time::local;

const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;

/// Whether `a` should run at `now`. A daily schedule runs in its hour, once for each time it
/// falls due: a run by hand at another time does not stand in for the next one.
pub fn is_due(a: &Automation, now: u64, offset: i64) -> bool {
    if !a.enabled {
        return false;
    }
    let since = a.last_run.map(|t| now.saturating_sub(t));
    let at = local(now, offset);
    // Time into the local hour: while `at.hour` is the schedule's, the hour began at its due time.
    let into_hour = ((now / 1000) as i64 + offset).rem_euclid(3600) as u64 * 1000 + now % 1000;
    let unrun = a.last_run.is_none_or(|t| t < now.saturating_sub(into_hour));
    match a.schedule {
        Schedule::EveryMinutes { minutes } => {
            since.is_none_or(|s| s >= u64::from(minutes.max(1)) * MINUTE)
        }
        Schedule::Hourly => since.is_none_or(|s| s >= HOUR),
        Schedule::Daily { hour } => at.hour == hour && unrun,
        Schedule::Weekdays { hour } => at.weekday < 5 && at.hour == hour && unrun,
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
        assert!(is_due(
            &auto(Schedule::Daily { hour: 9 }, Some(monday_9 - 10 * MINUTE)),
            monday_9,
            0
        ));
        assert!(!is_due(
            &auto(Schedule::Daily { hour: 10 }, None),
            monday_9,
            0
        ));
        let ran_by_hand = monday_9 - 18 * HOUR;
        assert!(is_due(
            &auto(Schedule::Daily { hour: 9 }, Some(ran_by_hand)),
            monday_9 + 10 * MINUTE,
            0
        ));
        assert!(!is_due(
            &auto(Schedule::Weekdays { hour: 9 }, Some(monday_9 - 20 * MINUTE)),
            monday_9,
            1800
        ));
        assert!(!is_due(
            &auto(Schedule::Daily { hour: 9 }, Some(monday_9 + 5 * MINUTE)),
            monday_9 + 50 * MINUTE,
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
