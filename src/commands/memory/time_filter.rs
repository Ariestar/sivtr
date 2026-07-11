use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Local, LocalResult, NaiveDate, TimeZone, Utc};
use sivtr_core::time::parse_timestamp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeRange {
    pub(crate) since: Option<DateTime<Utc>>,
    pub(crate) until: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimeBoundary {
    Since,
    Until,
}

impl TimeRange {
    pub(crate) fn contains_record_time(&self, timestamp: Option<&str>) -> bool {
        self.contains_timestamp(timestamp)
    }

    pub(crate) fn contains_timestamp(&self, timestamp: Option<&str>) -> bool {
        let Some(timestamp) = timestamp.and_then(parse_timestamp) else {
            return false;
        };

        if let Some(since) = self.since {
            if timestamp < since {
                return false;
            }
        }
        if let Some(until) = self.until {
            if timestamp > until {
                return false;
            }
        }
        true
    }
}

pub(crate) fn parse_duration_filter(value: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("--last requires a duration");
    }

    let duration =
        parse_duration(trimmed).with_context(|| format!("Invalid --last duration: {trimmed}"))?;
    Ok(now - duration)
}

pub(crate) fn build_time_range(
    since: Option<&str>,
    until: Option<&str>,
    last: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(Option<TimeRange>, Option<usize>)> {
    let mut since_time = match since {
        Some(value) => Some(
            parse_time_object(value, now, TimeBoundary::Since)
                .with_context(|| format!("Invalid --since time: {value}"))?,
        ),
        None => None,
    };
    let until_time = match until {
        Some(value) => Some(
            parse_time_object(value, now, TimeBoundary::Until)
                .with_context(|| format!("Invalid --until time: {value}"))?,
        ),
        None => None,
    };

    if let Some(value) = last {
        let last_since = parse_duration_filter(value, now)?;
        since_time = Some(since_time.map_or(last_since, |since| since.max(last_since)));
    }

    if let (Some(since), Some(until)) = (since_time, until_time) {
        if since > until {
            bail!("--since must be before or equal to --until");
        }
    }

    let range = if since_time.is_some() || until_time.is_some() {
        Some(TimeRange {
            since: since_time,
            until: until_time,
        })
    } else {
        None
    };

    Ok((range, None))
}

fn parse_time_object(
    value: &str,
    now: DateTime<Utc>,
    boundary: TimeBoundary,
) -> Result<DateTime<Utc>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("time value is empty");
    }

    if let Some(alias_time) = parse_time_alias(trimmed, now, boundary) {
        return Ok(alias_time);
    }

    if let Some(duration) = parse_duration(trimmed) {
        return Ok(now - duration);
    }

    parse_timestamp(trimmed).ok_or_else(|| anyhow!("unsupported time format"))
}

fn parse_time_alias(
    value: &str,
    now: DateTime<Utc>,
    boundary: TimeBoundary,
) -> Option<DateTime<Utc>> {
    let alias = normalize_alias(value);
    let today = now.with_timezone(&Local).date_naive();
    let (date, whole_day) = match alias.as_str() {
        "today" | "td" => (today, true),
        "yesterday" | "yd" => (today - Duration::days(1), true),
        "tomorrow" | "tmr" => (today + Duration::days(1), true),
        "day-before-yesterday" => (today - Duration::days(2), true),
        "this-morning" | "morning" => return local_time_on_date(today, 6, 0, 0),
        "this-afternoon" | "afternoon" => return local_time_on_date(today, 12, 0, 0),
        "this-evening" | "evening" => return local_time_on_date(today, 18, 0, 0),
        "tonight" => return local_time_on_date(today, 20, 0, 0),
        "now" => return Some(now),
        _ => return None,
    };

    if whole_day {
        match boundary {
            TimeBoundary::Since => local_start_of_day(date),
            TimeBoundary::Until => local_start_of_day(date + Duration::days(1)),
        }
    } else {
        None
    }
}

fn normalize_alias(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['_', ' '], "-")
}

fn local_start_of_day(date: NaiveDate) -> Option<DateTime<Utc>> {
    local_time_on_date(date, 0, 0, 0)
}

fn local_time_on_date(date: NaiveDate, hour: u32, min: u32, sec: u32) -> Option<DateTime<Utc>> {
    let datetime = date.and_hms_opt(hour, min, sec)?;
    match Local.from_local_datetime(&datetime) {
        LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(earliest, _) => Some(earliest.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

fn parse_duration(value: &str) -> Option<Duration> {
    let trimmed = value.trim();
    let number_end = trimmed
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .last()?;
    let amount = trimmed[..number_end].parse::<i64>().ok()?;
    if amount < 0 {
        return None;
    }
    let unit = trimmed[number_end..].trim().to_ascii_lowercase();
    match unit.as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => Some(Duration::seconds(amount)),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(Duration::minutes(amount)),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(Duration::hours(amount)),
        "d" | "day" | "days" => Some(Duration::days(amount)),
        "w" | "week" | "weeks" => Some(Duration::weeks(amount)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-23T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn parses_last_duration() {
        assert_eq!(
            parse_duration_filter("2h", now()).unwrap(),
            DateTime::parse_from_rfc3339("2026-05-23T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn parses_supported_timestamp_shapes() {
        assert_eq!(
            parse_timestamp("2026-05-23T12:00:00Z").unwrap(),
            DateTime::parse_from_rfc3339("2026-05-23T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(
            parse_timestamp("1779537600000").unwrap(),
            DateTime::parse_from_rfc3339("2026-05-23T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn combines_since_with_last_duration_using_newer_boundary() {
        let (range, recent_count) =
            build_time_range(Some("2026-05-23T09:00:00Z"), None, Some("2h"), now()).unwrap();

        assert_eq!(recent_count, None);
        assert_eq!(
            range.unwrap().since.unwrap(),
            DateTime::parse_from_rfc3339("2026-05-23T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn parses_today_alias_as_local_day_bounds() {
        let today = now().with_timezone(&Local).date_naive();
        let (range, _) = build_time_range(Some("today"), Some("today"), None, now()).unwrap();
        let range = range.unwrap();

        assert_eq!(range.since, local_start_of_day(today));
        assert_eq!(range.until, local_start_of_day(today + Duration::days(1)));
    }

    #[test]
    fn parses_yesterday_alias_as_previous_local_day() {
        let yesterday = now().with_timezone(&Local).date_naive() - Duration::days(1);
        let (range, _) =
            build_time_range(Some("yesterday"), Some("yesterday"), None, now()).unwrap();
        let range = range.unwrap();

        assert_eq!(range.since, local_start_of_day(yesterday));
        assert_eq!(
            range.until,
            local_start_of_day(yesterday + Duration::days(1))
        );
    }

    #[test]
    fn parses_human_time_aliases() {
        let today = now().with_timezone(&Local).date_naive();
        let (range, _) = build_time_range(Some("this morning"), None, None, now()).unwrap();

        assert_eq!(range.unwrap().since, local_time_on_date(today, 6, 0, 0));
    }
}
