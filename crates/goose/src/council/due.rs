//! Weekly due-window math. The loop ticks hourly; the session should land
//! Sunday 22:00 local onward, and skip if a success is less than six days old.

use chrono::{DateTime, Datelike, Duration, Local, Timelike, Weekday};

pub const PREFERRED_WEEKDAY: Weekday = Weekday::Sun;
pub const PREFERRED_HOUR: u32 = 22;
pub const MIN_GAP: Duration = Duration::days(6);

pub fn is_in_preferred_window(now: DateTime<Local>) -> bool {
    now.weekday() == PREFERRED_WEEKDAY && now.hour() >= PREFERRED_HOUR
}

/// A machine that slept through Sunday night should still run on Monday rather
/// than skipping the week. After Monday 22:00 we wait for the next Sunday.
pub fn is_catchup_window(now: DateTime<Local>) -> bool {
    match now.weekday() {
        Weekday::Sun => now.hour() >= PREFERRED_HOUR,
        Weekday::Mon => true,
        _ => false,
    }
}

pub fn too_soon(last_success: Option<DateTime<Local>>, now: DateTime<Local>) -> bool {
    match last_success {
        Some(ts) => now - ts < MIN_GAP,
        None => false,
    }
}

pub fn should_run(now: DateTime<Local>, last_success: Option<DateTime<Local>>) -> bool {
    if too_soon(last_success, now) {
        return false;
    }
    is_catchup_window(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn sunday_night_is_due_when_nothing_has_run() {
        // 2026-08-23 is a Sunday.
        let now = at(2026, 8, 23, 22);
        assert_eq!(now.weekday(), Weekday::Sun);
        assert!(should_run(now, None));
    }

    #[test]
    fn sunday_afternoon_waits() {
        let now = at(2026, 8, 23, 16);
        assert!(!should_run(now, None));
    }

    #[test]
    fn monday_catches_a_missed_sunday() {
        let now = at(2026, 8, 24, 9);
        assert_eq!(now.weekday(), Weekday::Mon);
        assert!(should_run(now, None));
    }

    #[test]
    fn tuesday_waits_for_next_sunday() {
        let now = at(2026, 8, 25, 10);
        assert!(!should_run(now, None));
    }

    #[test]
    fn skips_when_a_success_is_five_days_old() {
        let now = at(2026, 8, 23, 22);
        let last = at(2026, 8, 18, 22);
        assert!(too_soon(Some(last), now));
        assert!(!should_run(now, Some(last)));
    }

    #[test]
    fn runs_when_a_success_is_a_week_old() {
        let now = at(2026, 8, 23, 22);
        let last = at(2026, 8, 16, 22);
        assert!(!too_soon(Some(last), now));
        assert!(should_run(now, Some(last)));
    }
}
