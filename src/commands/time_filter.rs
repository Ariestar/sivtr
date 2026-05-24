use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeRange {
    pub(crate) since: Option<DateTime<Utc>>,
    pub(crate) until: Option<DateTime<Utc>>,
}

impl TimeRange {
    pub(crate) fn contains_record_time(&self, timestamp: Option<&str>) -> bool {
        self.contains_timestamp(timestamp)
    }

    pub(crate) fn contains_timestamp(&self, timestamp: Option<&str>) -> bool {
        let Some(timestamp) = timestamp
            .and_then(parse_timestamp)
            .or_else(|| timestamp.and_then(|value| parse_unix_timestamp(value.trim())))
        else {
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
            parse_time_object(value, now)
                .with_context(|| format!("Invalid --since time: {value}"))?,
        ),
        None => None,
    };
    let until_time = match until {
        Some(value) => Some(
            parse_time_object(value, now)
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

fn parse_time_object(value: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("time value is empty");
    }

    if let Some(duration) = parse_duration(trimmed) {
        return Ok(now - duration);
    }

    parse_timestamp(trimmed).ok_or_else(|| anyhow!("unsupported time format"))
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

pub(crate) fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(timestamp) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(timestamp.with_timezone(&Utc));
    }
    if let Some(timestamp) = parse_unix_timestamp(trimmed) {
        return Some(timestamp);
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let local = date.and_hms_opt(0, 0, 0)?;
        return local_to_utc(local);
    }
    if let Ok(datetime) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
        return local_to_utc(datetime);
    }
    if let Ok(datetime) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S") {
        return local_to_utc(datetime);
    }

    None
}

fn parse_unix_timestamp(value: &str) -> Option<DateTime<Utc>> {
    let number = value.parse::<i64>().ok()?;
    let seconds = if value.len() >= 13 {
        number / 1000
    } else {
        number
    };
    let nanos = if value.len() >= 13 {
        ((number % 1000).abs() as u32) * 1_000_000
    } else {
        0
    };
    Utc.timestamp_opt(seconds, nanos).single()
}

fn local_to_utc(datetime: NaiveDateTime) -> Option<DateTime<Utc>> {
    match Local.from_local_datetime(&datetime) {
        LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(earliest, _) => Some(earliest.with_timezone(&Utc)),
        LocalResult::None => None,
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
}
