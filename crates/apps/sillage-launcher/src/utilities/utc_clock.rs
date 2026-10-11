use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Timelike};

use super::UtcSnapshot;
use crate::errors::LauncherError;

impl UtcSnapshot {
    pub(crate) fn now() -> Result<Self, LauncherError> {
        snapshot_at(SystemTime::now())
    }
}

fn snapshot_at(time: SystemTime) -> Result<UtcSnapshot, LauncherError> {
    let (seconds, nanos) = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => (
            i64::try_from(duration.as_secs()).map_err(|_| unavailable())?,
            duration.subsec_nanos(),
        ),
        Err(error) => {
            let duration = error.duration();
            let seconds = i64::try_from(duration.as_secs())
                .map_err(|_| unavailable())?
                .checked_neg()
                .ok_or_else(unavailable)?;
            let nanos = duration.subsec_nanos();
            if nanos == 0 {
                (seconds, 0)
            } else {
                // Chrono uses a floor second plus a positive fractional remainder.
                (
                    seconds.checked_sub(1).ok_or_else(unavailable)?,
                    1_000_000_000 - nanos,
                )
            }
        }
    };
    snapshot(seconds, nanos)
}

fn snapshot(seconds: i64, nanos: u32) -> Result<UtcSnapshot, LauncherError> {
    let date = DateTime::from_timestamp(seconds, nanos).ok_or_else(unavailable)?;
    let year = date.year();
    if !(1..=9999).contains(&year) {
        return Err(unavailable());
    }
    // Chrono supplies bounded Gregorian fields; the existing tokens display whole UTC seconds.
    Ok(UtcSnapshot {
        year,
        month: date.month() as i32,
        day: date.day() as i32,
        hour: date.hour() as i32,
        minute: date.minute() as i32,
        second: date.second() as i32,
    })
}

fn unavailable() -> LauncherError {
    LauncherError::platform_unavailable("UTC clock value is unavailable")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::NaiveDate;

    use super::super::snippets;
    use super::*;

    fn at(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Result<SystemTime, &'static str> {
        let seconds = NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, second))
            .ok_or("Invalid fixture date")?
            .and_utc()
            .timestamp();
        let duration = Duration::from_secs(seconds.unsigned_abs());
        let time = if seconds < 0 {
            UNIX_EPOCH.checked_sub(duration)
        } else {
            UNIX_EPOCH.checked_add(duration)
        };
        time.ok_or("Fixture exceeds system clock range")
    }

    fn expand(time: SystemTime) -> Result<String, LauncherError> {
        snippets::expand("{datetime:UTC:ISO8601}", "", || snapshot_at(time))
    }

    #[test]
    fn pre_epoch_fraction_uses_previous_utc_second() -> Result<(), Box<dyn std::error::Error>> {
        let before = UNIX_EPOCH
            .checked_sub(Duration::from_nanos(100))
            .ok_or("Pre-epoch fixture unavailable")?;
        let after = UNIX_EPOCH
            .checked_add(Duration::from_nanos(999_999_900))
            .ok_or("Post-epoch fixture unavailable")?;
        assert_eq!(expand(before)?, "1969-12-31T23:59:59Z");
        assert_eq!(expand(after)?, "1970-01-01T00:00:00Z");
        let earlier = UNIX_EPOCH
            .checked_sub(Duration::new(1, 500_000_000))
            .ok_or("Earlier pre-epoch fixture unavailable")?;
        assert_eq!(expand(earlier)?, "1969-12-31T23:59:58Z");
        assert_eq!(
            snippets::expand(
                "{date:UTC:ISO8601}\r\n{time:UTC:ISO8601}\n{query}",
                "雪\n{date}",
                || snapshot_at(before),
            )?,
            "1969-12-31\r\n23:59:59Z\n雪\n{date}",
        );
        Ok(())
    }

    #[test]
    fn gregorian_leap_boundaries_reach_the_actual_snippet_consumer()
    -> Result<(), Box<dyn std::error::Error>> {
        for ((year, month, day), expected) in [
            ((1900, 2, 28), "1900-03-01T00:00:00Z"),
            ((2000, 2, 28), "2000-02-29T00:00:00Z"),
            ((2024, 2, 28), "2024-02-29T00:00:00Z"),
            ((2024, 2, 29), "2024-03-01T00:00:00Z"),
            ((2023, 12, 31), "2024-01-01T00:00:00Z"),
        ] {
            let boundary = at(year, month, day, 23, 59, 59)?
                .checked_add(Duration::from_secs(1))
                .ok_or("Leap boundary unavailable")?;
            assert_eq!(expand(boundary)?, expected);
        }
        Ok(())
    }

    #[test]
    fn iso_year_limits_fail_closed_without_widening_literal_tokens()
    -> Result<(), Box<dyn std::error::Error>> {
        // Test the ISO lower bound without requiring a platform clock era before 1601.
        let first = NaiveDate::from_ymd_opt(1, 1, 1)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .ok_or("Lower calendar fixture unavailable")?
            .and_utc()
            .timestamp();
        assert_eq!(
            snippets::expand("{datetime:UTC:ISO8601}", "", || snapshot(first, 0))?,
            "0001-01-01T00:00:00Z",
        );
        let last = at(9999, 12, 31, 23, 59, 59)?;
        assert_eq!(expand(last)?, "9999-12-31T23:59:59Z");
        let outside = last
            .checked_add(Duration::from_secs(1))
            .ok_or("Upper boundary unavailable")?;
        let earlier = first.checked_sub(1).ok_or("Lower boundary unavailable")?;
        let upper_error = expand(outside).expect_err("Upper ISO year is unsupported");
        assert_eq!(upper_error.code, "platform_unavailable");
        let lower_error = snippets::expand("{datetime:UTC:ISO8601}", "", || snapshot(earlier, 0))
            .expect_err("Lower ISO year is unsupported");
        assert_eq!(lower_error.code, "platform_unavailable");
        let unrepresentable =
            snippets::expand("{datetime:UTC:ISO8601}", "", || snapshot(i64::MAX, 0))
                .expect_err("Unrepresentable UTC clock value is unsupported");
        assert_eq!(unrepresentable.code, "platform_unavailable");
        let literal = "{date} {time:Local:ISO8601} {date:UTC:fr-FR}";
        assert_eq!(
            snippets::expand(literal, "", || snapshot_at(outside))?,
            literal
        );
        Ok(())
    }
}
