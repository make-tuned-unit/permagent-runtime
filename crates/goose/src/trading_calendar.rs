//! US cash-equity session helpers.
//!
//! NYSE regular hours close at 16:00 America/New_York. The Picker close scan
//! fires 30 minutes before that, weekdays that are not a listed holiday.

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};

/// Half hour before the 16:00 ET cash close.
pub const SCAN_HOUR: u32 = 15;
pub const SCAN_MINUTE: u32 = 30;

/// NYSE full-day closures we will not scan. Observed dates, not the civil
/// holiday when it falls on a weekend. Keep a couple of years ahead.
const NYSE_CLOSED: &[&str] = &[
    // 2025
    "2025-01-01",
    "2025-01-20",
    "2025-02-17",
    "2025-04-18",
    "2025-05-26",
    "2025-06-19",
    "2025-07-04",
    "2025-09-01",
    "2025-11-27",
    "2025-12-25",
    // 2026
    "2026-01-01",
    "2026-01-19",
    "2026-02-16",
    "2026-04-03",
    "2026-05-25",
    "2026-06-19",
    "2026-07-03",
    "2026-09-07",
    "2026-11-26",
    "2026-12-25",
    // 2027
    "2027-01-01",
    "2027-01-18",
    "2027-02-15",
    "2027-03-26",
    "2027-05-31",
    "2027-06-18",
    "2027-07-05",
    "2027-09-06",
    "2027-11-25",
    "2027-12-24",
];

pub fn is_us_dst(date: NaiveDate) -> bool {
    let start = nth_weekday(date.year(), 3, Weekday::Sun, 2);
    let end = nth_weekday(date.year(), 11, Weekday::Sun, 1);
    date >= start && date < end
}

fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: u8) -> NaiveDate {
    let mut d = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    while d.weekday() != weekday {
        d = d.succ_opt().expect("date");
    }
    d + chrono::Duration::days(7 * i64::from(n.saturating_sub(1)))
}

pub fn eastern_offset(date: NaiveDate) -> FixedOffset {
    let hours = if is_us_dst(date) { 4 } else { 5 };
    FixedOffset::west_opt(hours * 3600).expect("ET offset")
}

pub fn now_et(now: DateTime<Utc>) -> DateTime<FixedOffset> {
    let est = FixedOffset::west_opt(5 * 3600).expect("EST");
    let guess = now.with_timezone(&est).date_naive();
    now.with_timezone(&eastern_offset(guess))
}

pub fn is_trading_day(date: NaiveDate) -> bool {
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    let key = date.format("%Y-%m-%d").to_string();
    !NYSE_CLOSED.contains(&key.as_str())
}

pub fn scan_time() -> NaiveTime {
    NaiveTime::from_hms_opt(SCAN_HOUR, SCAN_MINUTE, 0).expect("15:30")
}

/// True when the close-scan should start (or catch up) for this instant.
pub fn should_scan(now: DateTime<Utc>) -> bool {
    let et = now_et(now);
    is_trading_day(et.date_naive()) && et.time() >= scan_time()
}

pub fn session_day(now: DateTime<Utc>) -> String {
    now_et(now).date_naive().format("%Y-%m-%d").to_string()
}

/// Sleep until the next 15:30 ET trading session, capped so a catch-up still
/// runs within a minute of boot.
pub fn sleep_until_next_window(now: DateTime<Utc>) -> chrono::Duration {
    if should_scan(now) {
        return chrono::Duration::seconds(60);
    }
    let et = now_et(now);
    let mut day = et.date_naive();
    if et.time() >= scan_time() {
        day = day.succ_opt().unwrap_or(day);
    }
    while !is_trading_day(day) {
        day = day.succ_opt().unwrap_or(day);
    }
    let fire = eastern_offset(day)
        .from_local_datetime(&day.and_time(scan_time()))
        .single()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(now + chrono::Duration::hours(1));
    let wait = fire.signed_duration_since(now);
    if wait <= chrono::Duration::zero() {
        chrono::Duration::seconds(60)
    } else {
        wait.min(chrono::Duration::minutes(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekends_are_not_trading_days() {
        let sat = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap();
        let sun = NaiveDate::from_ymd_opt(2026, 8, 23).unwrap();
        let mon = NaiveDate::from_ymd_opt(2026, 8, 24).unwrap();
        assert!(!is_trading_day(sat));
        assert!(!is_trading_day(sun));
        assert!(is_trading_day(mon));
    }

    #[test]
    fn good_friday_2026_is_closed() {
        assert!(!is_trading_day(
            NaiveDate::from_ymd_opt(2026, 4, 3).unwrap()
        ));
    }

    #[test]
    fn dst_brackets_are_correct() {
        assert!(!is_us_dst(NaiveDate::from_ymd_opt(2026, 3, 7).unwrap()));
        assert!(is_us_dst(NaiveDate::from_ymd_opt(2026, 3, 9).unwrap()));
        assert!(is_us_dst(NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()));
        assert!(!is_us_dst(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()));
    }

    #[test]
    fn scan_waits_until_half_hour_before_close() {
        // 2026-08-24 is a Monday. 19:00 UTC = 15:00 EDT — too early.
        let early = Utc.with_ymd_and_hms(2026, 8, 24, 19, 0, 0).unwrap();
        assert!(!should_scan(early));
        // 19:30 UTC = 15:30 EDT.
        let fire = Utc.with_ymd_and_hms(2026, 8, 24, 19, 30, 0).unwrap();
        assert!(should_scan(fire));
    }
}
