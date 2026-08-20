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
}
