//! Apple Calendar access through EventKit, asked for by the signed app.
//!
//! The user's report: "i cant find anywhere to grant access to my iCal so my
//! calendar can sync". There was nowhere. The daemon reads Calendar by sending
//! Apple events to Calendar.app (`osascript`, in goose-server's
//! `dashboard_cards.rs`), and macOS answers that with -1743 and no prompt —
//! partly because TCC attributes the request to whatever started `permagentd`,
//! which on the normal installed setup is launchd, a bare executable with no
//! Info.plist and nothing to put a prompt on.
//!
//! So the request comes from here instead. `Permagent.app` is a signed bundle
//! with `NSCalendarsFullAccessUsageDescription` in its Info.plist, so when this
//! process asks EventKit for access macOS shows the real system prompt, lists
//! Permagent under System Settings > Privacy & Security > Calendars, and lets
//! the user change their mind there later. The Home Calendar card calls these
//! commands directly; the daemon's AppleScript path is left alone.
//!
//! Follows the ObjC conventions in `accessibility.rs`: every call into
//! Objective-C is wrapped in `objc2::exception::catch`, and every failure
//! degrades to an answer the UI can render rather than to a crash. A calendar
//! we cannot read must never be an app that will not start.

use serde::Serialize;

/// One event as the Calendar card draws it. Times are Unix epoch
/// milliseconds so the webview formats them in the user's own locale.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub all_day: bool,
    /// The title of the calendar the event lives in ("Work", "Home", ...).
    pub calendar: String,
    pub location: Option<String>,
}

/// System Settings > Privacy & Security > Calendars. When access has been
/// denied, macOS will never prompt again, so this pane is the only way back —
/// the card links straight to it rather than describing where it is.
#[cfg(target_os = "macos")]
const PRIVACY_CALENDARS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars";

/// Current EventKit authorization for events, as the name the webview
/// switches on: `notDetermined`, `restricted`, `denied`, `fullAccess`,
/// `writeOnly`, or `unsupported` off macOS / on any failure to ask.
#[tauri::command]
pub fn calendar_authorization_status() -> String {
    #[cfg(target_os = "macos")]
    {
        macos::status_name(macos::status()).to_string()
    }
    #[cfg(not(target_os = "macos"))]
    {
        "unsupported".to_string()
    }
}

/// Ask for full access to events. Shows the system prompt the first time, and
/// the "upgrade to full access" prompt once for an app that holds write-only
/// access; after that macOS answers from the stored decision without asking, so this
/// resolves immediately with the current status. Resolves with the status
/// AFTER the request, in the same vocabulary as
/// `calendar_authorization_status` — or with `timedOut` when macOS never
/// answered (see `macos::TIMED_OUT`), which no status read ever returns.
#[tauri::command]
pub async fn request_calendar_access() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        // The completion handler fires only once the user answers the prompt,
        // which can take minutes. Waiting for it on a blocking-pool thread
        // keeps it off both the AppKit main thread and the async executor.
        tauri::async_runtime::spawn_blocking(macos::request_access_blocking)
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok("unsupported".to_string())
    }
}

/// Events from the start of today through the end of day `days` (1 = today
/// only), across every calendar the user has in Calendar.app, sorted by start.
/// Errors when access has not been granted, so an empty list always means
/// "nothing scheduled" and never "not allowed to look".
#[tauri::command]
pub async fn calendar_upcoming_events(days: Option<u32>) -> Result<Vec<CalendarEvent>, String> {
    #[cfg(target_os = "macos")]
    {
        let days = days.unwrap_or(1).clamp(1, 31);
        tauri::async_runtime::spawn_blocking(move || macos::upcoming_events(days))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = days;
        Err("Calendar access needs macOS".to_string())
    }
}

/// Open the Calendars privacy pane. A fixed URL — nothing from the webview
/// reaches the command line. Done natively because the shell plugin's `open`
/// scope only admits http(s)/mailto/tel, and would refuse this scheme.
#[tauri::command]
pub fn open_calendar_privacy_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .arg(PRIVACY_CALENDARS_URL)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Calendar settings are macOS-only".to_string())
    }
}

/// Unix-second bounds of `[local midnight today, local midnight today + days)`.
/// Starting at midnight rather than now keeps this morning's meetings on the
/// card for the rest of the day, matching what the daemon's card showed.
#[cfg(any(target_os = "macos", test))]
fn day_window(days: u32) -> (i64, i64) {
    use chrono::{Days, Local, NaiveDate, TimeZone};

    fn local_midnight(date: NaiveDate) -> Option<i64> {
        let naive = date.and_hms_opt(0, 0, 0)?;
        // `earliest` covers the DST-gap day where midnight is ambiguous.
        Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|d| d.timestamp())
    }

    let today = Local::now().date_naive();
    let start = local_midnight(today).unwrap_or_else(|| Local::now().timestamp());
    let end = today
        .checked_add_days(Days::new(u64::from(days.max(1))))
        .and_then(local_midnight)
        .unwrap_or(start + i64::from(days.max(1)) * 86_400);
    (start, end)
}

#[cfg(target_os = "macos")]
mod macos {
    use super::CalendarEvent;
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
    use objc2_foundation::{NSDate, NSError};
    use std::panic::AssertUnwindSafe;
    use std::sync::mpsc;
    use std::time::Duration;

    /// How long to wait for the user to answer the system prompt before
    /// giving up and reporting `TIMED_OUT`.
    const PROMPT_WAIT: Duration = Duration::from_secs(300);

    /// What the request reports when the completion handler never fired: the
    /// prompt went unanswered for `PROMPT_WAIT` (the screen locked, the user
    /// walked away), or EventKit dropped the handler without calling it.
    /// This used to report the unchanged status instead, and from write-only
    /// that is indistinguishable from macOS answering "write-only again" — so
    /// the card retired Allow Full Access for System Settings although nobody
    /// had seen the prompt — the case a locked screen during QA exposed.
    /// `calendarAccess.ts` (`REQUEST_TIMED_OUT`) spells the same string.
    pub const TIMED_OUT: &str = "timedOut";

    /// A sentinel for "could not ask" — not an EventKit value.
    const UNKNOWN: EKAuthorizationStatus = EKAuthorizationStatus(-1);

    pub fn status() -> EKAuthorizationStatus {
        objc2::exception::catch(AssertUnwindSafe(|| unsafe {
            EKEventStore::authorizationStatusForEntityType(EKEntityType::Event)
        }))
        .unwrap_or(UNKNOWN)
    }

    /// Compared by value rather than matched: on macOS 13 and earlier the
    /// value 3 is spelled `Authorized`, on 14+ `FullAccess` — same number,
    /// same meaning for reading events.
    pub fn status_name(s: EKAuthorizationStatus) -> &'static str {
        if s == EKAuthorizationStatus::NotDetermined {
            "notDetermined"
        } else if s == EKAuthorizationStatus::Restricted {
            "restricted"
        } else if s == EKAuthorizationStatus::Denied {
            "denied"
        } else if s == EKAuthorizationStatus::FullAccess {
            "fullAccess"
        } else if s == EKAuthorizationStatus::WriteOnly {
            "writeOnly"
        } else {
            "unsupported"
        }
    }

    /// Whether a full-access request can still raise a prompt from `before`.
    ///
    /// `NotDetermined` gets the first-time prompt. `WriteOnly` gets the
    /// upgrade prompt: TN3153 ("An app with write-only access may later
    /// request full access") and the `requestFullAccessToEvents` docs ("If
    /// your app ... only has write-only access to events, you must request
    /// full access") both route write-only through this same request. The
    /// installed app sat in write-only with the card offering only "Open
    /// System Settings", because this used to return early for every status
    /// but `NotDetermined` — so the one prompt macOS would still show was
    /// never asked for. Denied and restricted never prompt again.
    pub fn can_prompt(before: EKAuthorizationStatus) -> bool {
        before == EKAuthorizationStatus::NotDetermined || before == EKAuthorizationStatus::WriteOnly
    }

    pub fn request_access_blocking() -> Result<String, String> {
        let before = status();
        if !can_prompt(before) {
            // Every request from here completes at once with the stored
            // answer, so there is nothing to wait for — the card turns a
            // denial into a link to System Settings instead.
            return Ok(status_name(before).to_string());
        }

        let (tx, rx) = mpsc::channel::<bool>();
        let caught = objc2::exception::catch(AssertUnwindSafe(move || unsafe {
            let store = EKEventStore::new();
            let block: RcBlock<dyn Fn(Bool, *mut NSError)> =
                RcBlock::new(move |granted: Bool, _error: *mut NSError| {
                    let _ = tx.send(granted.as_bool());
                });
            if objc2::available!(macos = 14.0) {
                // From write-only this is the upgrade request; macOS answers
                // at once without a prompt if it has already asked for full
                // access, and the status read below reports that.
                store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&block));
            } else {
                // Before macOS 14 there is no full/write-only split; this is
                // the one request that exists there (and `WriteOnly` cannot be
                // the starting status, so only `NotDetermined` reaches it).
                request_legacy(&store, RcBlock::as_ptr(&block));
            }
            // EventKit copies the block, so ours may drop here. The store is
            // returned and held until the answer arrives.
            store
        }));
        let _store = caught.map_err(|_| "EventKit refused the access request".to_string())?;

        // The boolean alone cannot distinguish "denied" from "write-only";
        // the status read afterwards can. No answer at all is reported as
        // such, never as a status: see `TIMED_OUT`.
        match rx.recv_timeout(PROMPT_WAIT) {
            Ok(_granted) => Ok(status_name(status()).to_string()),
            Err(_) => Ok(TIMED_OUT.to_string()),
        }
    }

    /// The pre-macOS-14 request, isolated so its deprecation allowance
    /// covers exactly this one call.
    #[allow(deprecated)]
    unsafe fn request_legacy(
        store: &EKEventStore,
        completion: *mut block2::DynBlock<dyn Fn(Bool, *mut NSError)>,
    ) {
        store.requestAccessToEntityType_completion(EKEntityType::Event, completion);
    }

    pub fn upcoming_events(days: u32) -> Result<Vec<CalendarEvent>, String> {
        let current = status();
        if current != EKAuthorizationStatus::FullAccess {
            return Err(format!("Calendar access is {}", status_name(current)));
        }
        let (start_secs, end_secs) = super::day_window(days);

        let caught = objc2::exception::catch(AssertUnwindSafe(|| unsafe {
            let store = EKEventStore::new();
            let start = NSDate::dateWithTimeIntervalSince1970(start_secs as f64);
            let end = NSDate::dateWithTimeIntervalSince1970(end_secs as f64);
            let predicate =
                store.predicateForEventsWithStartDate_endDate_calendars(&start, &end, None);
            let found = store.eventsMatchingPredicate(&predicate);

            let mut out: Vec<CalendarEvent> = Vec::new();
            for event in found.to_vec() {
                let calendar = event
                    .calendar()
                    .map(|c| c.title().to_string())
                    .unwrap_or_default();
                let location = event
                    .location()
                    .map(|l| l.to_string())
                    .filter(|l| !l.trim().is_empty());
                out.push(CalendarEvent {
                    title: event.title().to_string(),
                    start_ms: (event.startDate().timeIntervalSince1970() * 1000.0) as i64,
                    end_ms: (event.endDate().timeIntervalSince1970() * 1000.0) as i64,
                    all_day: event.isAllDay(),
                    calendar,
                    location,
                });
            }
            out
        }));

        let mut events = caught.map_err(|_| "Calendar could not be read".to_string())?;
        // EventKit returns matches in no guaranteed order.
        events.sort_by_key(|e| (e.start_ms, e.end_ms));
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::day_window;

    #[test]
    fn day_window_starts_at_or_before_now_and_spans_the_requested_days() {
        let now = chrono::Local::now().timestamp();
        let (start, end) = day_window(1);
        assert!(start <= now, "window must include the rest of today");
        assert!(end > now, "window must reach past now");
        // A local day is 23–25 hours across DST changes.
        assert!((23 * 3600..=25 * 3600).contains(&(end - start)));

        let (start7, end7) = day_window(7);
        assert_eq!(start7, start);
        assert!(end7 - start7 >= 7 * 23 * 3600);
    }

    #[test]
    fn day_window_treats_zero_days_as_today() {
        assert_eq!(day_window(0), day_window(1));
    }
}
