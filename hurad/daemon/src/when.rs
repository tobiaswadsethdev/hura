//! Reading `hurad new --at`: when a person means, as seconds since the epoch.
//!
//! In this machine's local time, because the person typing it is usually
//! sitting at it, and a server three time zones away is the rarer case -- the
//! apps send an instant, worked out where the person is.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

/// What `--at` accepts, for the error that says it did not.
pub const FORMS: &str = "`02:00`, `2026-11-20 14:30`, `+90m`, `+2h` or `+1d`";

/// `text` read against `now`: a time of day is its next occurrence, a date
/// and time is that minute, and `+` an amount is that long from now.
pub fn parse<Tz: TimeZone>(text: &str, now: DateTime<Tz>) -> Result<u64, String> {
    let text = text.trim();
    let bad = || format!("`{text}` is not a time; try {FORMS}");

    if let Some(amount) = text.strip_prefix('+') {
        let (digits, unit) = amount.split_at(amount.trim_end_matches(char::is_alphabetic).len());
        let n: i64 = digits.parse().map_err(|_| bad())?;
        let span = match unit {
            "s" => Duration::seconds(n),
            "m" | "min" => Duration::minutes(n),
            "h" => Duration::hours(n),
            "d" => Duration::days(n),
            _ => return Err(bad()),
        };
        return Ok((now + span).timestamp().max(0) as u64);
    }

    if let Ok(at) = DateTime::parse_from_rfc3339(text) {
        return Ok(at.timestamp().max(0) as u64);
    }

    let tz = now.timezone();
    let local = |naive: NaiveDateTime| {
        tz.from_local_datetime(&naive)
            .earliest()
            .map(|t| t.timestamp().max(0) as u64)
            .ok_or_else(|| format!("`{text}` does not happen here: the clocks change over it"))
    };

    for form in ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, form) {
            return local(naive);
        }
    }
    if let Ok(day) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return local(day.and_time(NaiveTime::MIN));
    }

    for form in ["%H:%M", "%H:%M:%S"] {
        if let Ok(time) = NaiveTime::parse_from_str(text, form) {
            let today = now.naive_local().date();
            let at = local(today.and_time(time))?;
            // Already past today means tomorrow: nobody schedules a start
            // for a minute that has gone.
            if at as i64 > now.timestamp() {
                return Ok(at);
            }
            return local(today.succ_opt().ok_or_else(bad)?.and_time(time));
        }
    }
    Err(bad())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    /// 2026-11-16 22:15:00 at UTC+1.
    fn now() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(3600)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 16, 22, 15, 0)
            .unwrap()
    }

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> u64 {
        now()
            .timezone()
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp() as u64
    }

    #[test]
    fn a_time_of_day_is_its_next_occurrence() {
        assert_eq!(parse("23:00", now()), Ok(at(2026, 11, 16, 23, 0)));
        assert_eq!(parse("02:00", now()), Ok(at(2026, 11, 17, 2, 0)));
        assert_eq!(parse("22:15", now()), Ok(at(2026, 11, 17, 22, 15)));
    }

    #[test]
    fn a_date_and_time_is_that_minute_here() {
        assert_eq!(
            parse("2026-11-20 14:30", now()),
            Ok(at(2026, 11, 20, 14, 30))
        );
        assert_eq!(
            parse("2026-11-20T14:30", now()),
            Ok(at(2026, 11, 20, 14, 30))
        );
        assert_eq!(parse("2026-11-20", now()), Ok(at(2026, 11, 20, 0, 0)));
    }

    #[test]
    fn an_amount_is_from_now() {
        let base = now().timestamp() as u64;
        assert_eq!(parse("+90m", now()), Ok(base + 90 * 60));
        assert_eq!(parse("+2h", now()), Ok(base + 2 * 3600));
        assert_eq!(parse("+1d", now()), Ok(base + 86_400));
    }

    #[test]
    fn an_instant_with_its_offset_is_taken_as_it_is() {
        assert_eq!(
            parse("2026-11-17T01:00:00Z", now()),
            Ok(at(2026, 11, 17, 2, 0))
        );
    }

    #[test]
    fn anything_else_says_what_would_do() {
        for text in ["tomorrow", "25:00", "+3w", "+h", ""] {
            let e = parse(text, now()).unwrap_err();
            assert!(e.contains("02:00"), "{text}: {e}");
        }
    }
}
