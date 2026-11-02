//! Small display helpers shared by the views.

use chrono::{DateTime, FixedOffset, Utc};

/// `Ada Lovelace <ada@example.com>` → `Ada Lovelace`; a bare address →
/// its local part.
pub fn display_name(addr: &str) -> String {
    let addr = addr.trim();
    if let Some((name, _)) = addr.split_once('<') {
        let name = name.trim().trim_matches('"').trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    let email = addr.trim_start_matches('<').trim_end_matches('>');
    email.split('@').next().unwrap_or(email).to_string()
}

/// Compact relative time: `now`, `5m`, `3h`, `2d`, then a date.
pub fn ago(d: DateTime<FixedOffset>, now: DateTime<Utc>) -> String {
    let secs = (now - d.with_timezone(&Utc)).num_seconds();
    match secs {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 30 * 86_400 => format!("{}d", s / 86_400),
        _ => d.format("%b %-d, %Y").to_string(),
    }
}

/// `2026-11-02 14:05`.
pub fn date_time(d: DateTime<FixedOffset>) -> String {
    d.format("%Y-%m-%d %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            display_name("Ada Lovelace <ada@example.com>"),
            "Ada Lovelace"
        );
        assert_eq!(display_name("\"Bob\" <bob@example.com>"), "Bob");
        assert_eq!(display_name("<carol@example.com>"), "carol");
        assert_eq!(display_name("dave@example.com"), "dave");
    }

    #[test]
    fn relative_times() {
        let now: DateTime<Utc> = "2026-11-02T12:00:00Z".parse().unwrap();
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap();
        assert_eq!(ago(at("2026-11-02T11:59:30Z"), now), "now");
        assert_eq!(ago(at("2026-11-02T11:55:00Z"), now), "5m");
        assert_eq!(ago(at("2026-11-02T14:00:00+05:00"), now), "3h");
        assert_eq!(ago(at("2026-10-31T12:00:00Z"), now), "2d");
        assert_eq!(ago(at("2025-03-04T12:00:00Z"), now), "Mar 4, 2025");
    }
}
