use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The local zone's offset from UTC in seconds, as the system reports it now.
pub fn local_offset_secs() -> i64 {
    i64::from(chrono::Local::now().offset().local_minus_utc())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub day: i64,
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
}

/// Splits a UTC millisecond time into local calendar parts. Weekday 0 is Monday.
pub fn local(ms: u64, offset: i64) -> Local {
    let secs = (ms / 1000) as i64 + offset;
    let day = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    Local {
        day,
        weekday: ((day + 3).rem_euclid(7)) as u8,
        hour: (rem / 3600) as u8,
        minute: ((rem % 3600) / 60) as u8,
    }
}

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

pub fn clock(l: Local, h24: bool) -> String {
    if h24 {
        format!("{}:{:02}", l.hour, l.minute)
    } else {
        let h = match l.hour % 12 {
            0 => 12,
            h => h,
        };
        format!(
            "{h}:{:02} {}",
            l.minute,
            if l.hour < 12 { "AM" } else { "PM" }
        )
    }
}

/// A message time stamp: the clock today, the weekday within a week, else the age in days.
pub fn stamp(at: u64, now: u64, offset: i64, h24: bool) -> String {
    let (a, n) = (local(at, offset), local(now, offset));
    match n.day - a.day {
        0 => clock(a, h24),
        1 => format!("Yesterday {}", clock(a, h24)),
        2..=6 => format!("{} {}", WEEKDAYS[a.weekday as usize], clock(a, h24)),
        _ => format!("{} days ago", n.day - a.day),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bucket {
    Today,
    Yesterday,
    Earlier,
}

pub fn bucket(at: u64, now: u64, offset: i64) -> Bucket {
    match local(now, offset).day - local(at, offset).day {
        i64::MIN..=0 => Bucket::Today,
        1 => Bucket::Yesterday,
        _ => Bucket::Earlier,
    }
}

pub fn ago(at: u64, now: u64) -> String {
    let s = now.saturating_sub(at) / 1000;
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

pub fn duration(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m {}s", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400_000;

    #[test]
    fn epoch_was_a_thursday_and_offsets_shift_the_day() {
        assert_eq!(local(0, 0).weekday, 3);
        assert_eq!(local(0, -3600).day, -1);
        assert_eq!(local(0, -3600).hour, 23);
    }

    #[test]
    fn stamps_and_buckets() {
        let now = 20 * DAY + 10 * 3_600_000;
        assert_eq!(stamp(now - 60_000, now, 0, true), "9:59");
        assert_eq!(stamp(now - DAY, now, 0, false), "Yesterday 10:00 AM");
        assert_eq!(stamp(now - 3 * DAY, now, 0, true), "Sunday 10:00");
        assert_eq!(bucket(now - DAY, now, 0), Bucket::Yesterday);
        assert_eq!(bucket(now - 9 * DAY, now, 0), Bucket::Earlier);
        assert_eq!(ago(now - 7_200_000, now), "2h");
        assert_eq!(duration(260_000), "4m 20s");
    }
}
