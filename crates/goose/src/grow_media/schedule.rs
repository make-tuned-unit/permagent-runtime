//! Send-time recommendation for a social_post.
//!
//! Uses the user's local timezone (this is a desktop app) and generic
//! channel windows. Winning hours from *this* project's live insights can
//! be threaded in later; nothing here is a particular audience's habit.

use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Timelike, Utc, Weekday};

pub struct ScheduleInput<'a> {
    pub channel: &'a str,
    pub occupied: &'a [DateTime<Utc>],
    pub not_before: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
}

/// Hour (0–23) in the user's local clock that we aim for, by channel slug.
fn target_hour(channel: &str) -> u32 {
    match channel {
        "ig" | "instagram" | "tiktok" | "reels" => 11,
        "li" | "linkedin" => 9,
        "x" | "twitter" => 10,
        _ => 10,
    }
}

fn prefers_weekdays(channel: &str) -> bool {
    matches!(channel, "li" | "linkedin")
}

pub fn recommend_scheduled_for(input: ScheduleInput<'_>) -> DateTime<Utc> {
    let floor = input.now + Duration::hours(2);
    let beat_floor = input.not_before.unwrap_or(input.now);
    let min = if beat_floor > floor {
        beat_floor
    } else {
        floor
    };

    // Start at the first channel window on or after `min`, not `now`.
    // Stepping from now in 2h increments only looks ~4 days out and would
    // miss a later beat floor.
    let mut candidate = next_window(min.with_timezone(&Local), input.channel);
    if candidate.with_timezone(&Utc) < min {
        candidate = next_window(candidate + Duration::hours(1), input.channel);
    }

    for _ in 0..48 {
        let utc = candidate.with_timezone(&Utc);
        if utc >= min && !conflicts(utc, input.occupied) {
            return utc;
        }
        candidate += Duration::hours(2);
        if prefers_weekdays(input.channel) && is_weekend(candidate) {
            candidate = next_window(candidate, input.channel);
        }
    }
    candidate.with_timezone(&Utc)
}

fn is_weekend(dt: DateTime<Local>) -> bool {
    matches!(dt.weekday(), Weekday::Sat | Weekday::Sun)
}

fn next_window(from: DateTime<Local>, channel: &str) -> DateTime<Local> {
    let hour = target_hour(channel);
    let mut day = from.date_naive();
    if from.hour() >= hour {
        day += Duration::days(1);
    }
    if prefers_weekdays(channel) {
        while matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            day += Duration::days(1);
        }
    }
    let naive = day.and_hms_opt(hour, 0, 0).expect("valid hms");
    Local
        .from_local_datetime(&naive)
        .single()
        .or_else(|| Local.from_local_datetime(&naive).earliest())
        .unwrap_or(from + Duration::hours(24))
}

/// Render a stored send time on a given clock for the model to repeat to the
/// user, e.g. `Fri Sep 25, 11:00 AM (UTC-03:00)`.
///
/// `scheduledFor` is stored as RFC-3339 UTC because the UI and the publisher
/// parse it. Handing the model ONLY that string made it read "14:00" back as
/// the user's time while the calendar showed 11:00 AM. The clock is a
/// parameter so tests do not depend on the machine's zone; callers pass
/// [`Local`]. chrono has no zone abbreviations without a tz database, so the
/// UTC offset is spelled out instead.
pub fn format_send_time_in<Tz>(when: DateTime<Utc>, tz: &Tz) -> String
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    when.with_timezone(tz)
        .format("%a %b %-d, %-I:%M %p (UTC%:z)")
        .to_string()
}

/// [`format_send_time_in`] for a stored RFC-3339 string. `None` when the
/// string does not parse — the caller then simply omits the local field.
pub fn scheduled_for_local_in<Tz>(rfc3339: &str, tz: &Tz) -> Option<String>
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|d| format_send_time_in(d.with_timezone(&Utc), tz))
}

/// The user's clock: this is a desktop app, so the machine's zone is theirs.
pub fn scheduled_for_local(rfc3339: &str) -> Option<String> {
    scheduled_for_local_in(rfc3339, &Local)
}

fn conflicts(when: DateTime<Utc>, occupied: &[DateTime<Utc>]) -> bool {
    occupied.iter().any(|other| {
        let delta = if when >= *other {
            when - *other
        } else {
            *other - when
        };
        delta < Duration::minutes(45)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn recommendation_is_in_the_future_and_avoids_occupied() {
        let now = utc(2026, 8, 20, 12);
        let taken = recommend_scheduled_for(ScheduleInput {
            channel: "li",
            occupied: &[],
            not_before: None,
            now,
        });
        assert!(taken >= now + Duration::hours(2));
        let next = recommend_scheduled_for(ScheduleInput {
            channel: "li",
            occupied: &[taken],
            not_before: None,
            now,
        });
        assert!(next >= taken + Duration::minutes(45) || next <= taken - Duration::minutes(45));
        assert_ne!(next, taken);
    }

    #[test]
    fn beat_order_floor_is_honored() {
        let now = utc(2026, 8, 20, 12);
        let floor = utc(2026, 8, 25, 15);
        let rec = recommend_scheduled_for(ScheduleInput {
            channel: "ig",
            occupied: &[],
            not_before: Some(floor),
            now,
        });
        assert!(rec >= floor);
    }

    #[test]
    fn send_time_renders_on_the_given_clock_not_utc() {
        // 14:00 UTC is 11:00 AM on a UTC-3 clock — the calendar's value.
        let west3 = chrono::FixedOffset::west_opt(3 * 3600).unwrap();
        assert_eq!(
            format_send_time_in(utc(2026, 9, 25, 14), &west3),
            "Fri Sep 25, 11:00 AM (UTC-03:00)"
        );
        let east9 = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(
            format_send_time_in(utc(2026, 9, 25, 14), &east9),
            "Fri Sep 25, 11:00 PM (UTC+09:00)"
        );
    }

    #[test]
    fn stored_rfc3339_parses_to_local_and_garbage_is_none() {
        let west3 = chrono::FixedOffset::west_opt(3 * 3600).unwrap();
        assert_eq!(
            scheduled_for_local_in("2026-09-25T14:00:00+00:00", &west3).as_deref(),
            Some("Fri Sep 25, 11:00 AM (UTC-03:00)")
        );
        // Crossing midnight moves the day too.
        assert_eq!(
            scheduled_for_local_in("2026-09-25T01:30:00Z", &west3).as_deref(),
            Some("Thu Sep 24, 10:30 PM (UTC-03:00)")
        );
        assert_eq!(scheduled_for_local_in("tomorrow at 2", &west3), None);
    }
}
