//! Friendly date input → RFC 5322 on disk.
//!
//! Accepted: `YYYY-MM-DD`, `today`, `tomorrow`, `+Nd`, `+Nw`, weekday names
//! (`monday`/`mon`: the next such day, 1–7 days ahead), and full RFC 5322 /
//! RFC 3339 dates.

use crate::error::{Error, Result};
use chrono::{
    DateTime, Datelike, Days, FixedOffset, Local, NaiveDate, NaiveTime, TimeZone, Weekday,
};

const VALID_DATES: &str =
    "YYYY-MM-DD, today, tomorrow, +Nd, +Nw, monday..sunday (or mon..sun), RFC 5322";

fn invalid(input: &str) -> Error {
    Error::validation("invalid-date")
        .with("Value", input)
        .with("Valid", VALID_DATES)
}

fn weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

/// Resolve to a calendar day relative to `today`.
pub fn parse_day(input: &str, today: NaiveDate) -> Option<NaiveDate> {
    let s = input.trim().to_lowercase();
    match s.as_str() {
        "today" => return Some(today),
        "tomorrow" => return today.checked_add_days(Days::new(1)),
        _ => {}
    }
    if let Some(rest) = s.strip_prefix('+') {
        let (num, unit) = rest.split_at(rest.len().saturating_sub(1));
        let n: u64 = num.parse().ok()?;
        return match unit {
            "d" => today.checked_add_days(Days::new(n)),
            "w" => today.checked_add_days(Days::new(n * 7)),
            _ => None,
        };
    }
    if let Some(wd) = weekday(&s) {
        let cur = today.weekday().num_days_from_monday();
        let delta = (wd.num_days_from_monday() + 6 - cur) % 7 + 1;
        return today.checked_add_days(Days::new(u64::from(delta)));
    }
    NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()
}

/// Parse friendly input into an RFC 5322 timestamp (local midnight for days).
pub fn parse(input: &str, now: DateTime<Local>) -> Result<DateTime<FixedOffset>> {
    if let Ok(d) = DateTime::parse_from_rfc2822(input.trim()) {
        return Ok(d);
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(input.trim()) {
        return Ok(d);
    }
    let day = parse_day(input, now.date_naive()).ok_or_else(|| invalid(input))?;
    let naive = day.and_time(NaiveTime::MIN);
    let local = Local
        .from_local_datetime(&naive)
        .earliest()
        .ok_or_else(|| invalid(input))?;
    Ok(local.fixed_offset())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(); // Saturday
        let d = |s| parse_day(s, today).map(|d| d.to_string());
        assert_eq!(d("today").unwrap(), "2026-10-31");
        assert_eq!(d("tomorrow").unwrap(), "2026-11-01");
        assert_eq!(d("+3d").unwrap(), "2026-11-03");
        assert_eq!(d("+2w").unwrap(), "2026-11-14");
        assert_eq!(d("monday").unwrap(), "2026-11-02");
        assert_eq!(d("Sat").unwrap(), "2026-11-07");
        assert_eq!(d("2027-01-05").unwrap(), "2027-01-05");
        assert!(d("someday").is_none());
        assert!(d("+3x").is_none());
    }
}
