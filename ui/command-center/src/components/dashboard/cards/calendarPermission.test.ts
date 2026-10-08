/**
 * The macOS keys without which a Calendar permission prompt can never appear,
 * and the shell commands the Calendar card calls to raise one.
 *
 * Two read paths exist. The daemon reads today's events by sending Apple
 * events to Calendar.app (`osascript` + `tell application "Calendar"`, in
 * goose-server's `dashboard_cards.rs`) — Automation, whose keys are
 * NSAppleEventsUsageDescription and `com.apple.security.automation.apple-events`.
 * That path can never prompt when launchd owns permagentd, which is how the
 * user ended up reporting "i cant find anywhere to grant access to my iCal".
 *
 * So the signed app now asks EventKit itself (`src-tauri/src/calendar.rs`).
 * That needs NSCalendarsFullAccessUsageDescription (macOS 14+),
 * NSCalendarsUsageDescription (13 and earlier) and the hardened-runtime
 * entitlement `com.apple.security.personal-information.calendars`. With a
 * usage description absent, macOS does not show a prompt — it kills the app.
 *
 * Node environment (not jsdom) so `import.meta.url` is a real file URL.
 */
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const INFO_PLIST = fileURLToPath(
  new URL('../../../../../desktop/src-tauri/Info.plist', import.meta.url),
);
const ENTITLEMENTS = fileURLToPath(
  new URL('../../../../../desktop/src-tauri/Entitlements.plist', import.meta.url),
);
const MAIN_RS = fileURLToPath(
  new URL('../../../../../desktop/src-tauri/src/main.rs', import.meta.url),
);
const CALENDAR_RS = fileURLToPath(
  new URL('../../../../../desktop/src-tauri/src/calendar.rs', import.meta.url),
);
const CARGO_TOML = fileURLToPath(
  new URL('../../../../../desktop/src-tauri/Cargo.toml', import.meta.url),
);

/** The commands `useShellCalendar.ts` invokes, by name. */
const SHELL_COMMANDS = [
  'calendar_authorization_status',
  'request_calendar_access',
  'calendar_upcoming_events',
  'open_calendar_privacy_settings',
  // The range-based Calendar card (useCalendarEvents / EventPopover).
  'calendar_events_in_range',
  'open_calendar_app',
];

function purposeOf(plist: string, key: string): string {
  const m = new RegExp(`<key>${key}</key>\\s*<string>([^<]+)</string>`).exec(plist);
  return m?.[1]?.trim() ?? '';
}

describe('macOS calendar permission keys', () => {
  it('declares NSAppleEventsUsageDescription with a real purpose string', () => {
    const plist = readFileSync(INFO_PLIST, 'utf8');
    expect(plist).toContain('<key>NSAppleEventsUsageDescription</key>');
    // macOS shows this string in the prompt; an empty one is a rejected build.
    const purpose = /<key>NSAppleEventsUsageDescription<\/key>\s*<string>([^<]+)<\/string>/
      .exec(plist);
    expect(purpose?.[1]?.trim().length ?? 0).toBeGreaterThan(10);
  });

  it('carries the hardened-runtime entitlement that lets the app send Apple events', () => {
    const ents = readFileSync(ENTITLEMENTS, 'utf8');
    expect(ents).toMatch(
      /<key>com\.apple\.security\.automation\.apple-events<\/key>\s*<true\/>/,
    );
  });

  it('declares both EventKit calendar usage descriptions with real purpose strings', () => {
    const plist = readFileSync(INFO_PLIST, 'utf8');
    // macOS 14+ (requestFullAccessToEventsWithCompletion) and 13- (requestAccessToEntityType).
    expect(purposeOf(plist, 'NSCalendarsFullAccessUsageDescription').length).toBeGreaterThan(10);
    expect(purposeOf(plist, 'NSCalendarsUsageDescription').length).toBeGreaterThan(10);
  });

  it('carries the hardened-runtime calendars entitlement for EventKit', () => {
    const ents = readFileSync(ENTITLEMENTS, 'utf8');
    expect(ents).toMatch(
      /<key>com\.apple\.security\.personal-information\.calendars<\/key>\s*<true\/>/,
    );
  });

  it('lets a write-only app reach the full-access request instead of returning early', () => {
    // The installed app reported writeOnly and the request returned before
    // asking, so the upgrade prompt macOS still offers was never shown. This
    // reads the Rust source; it proves the gate admits WriteOnly and that the
    // request path consults it, not that macOS then shows the prompt.
    const calendar = readFileSync(CALENDAR_RS, 'utf8');
    const gate = /pub fn can_prompt\(before: EKAuthorizationStatus\) -> bool \{([\s\S]*?)\n    \}/.exec(calendar);
    expect(gate?.[1]).toContain('EKAuthorizationStatus::NotDetermined');
    expect(gate?.[1]).toContain('EKAuthorizationStatus::WriteOnly');
    const request = /pub fn request_access_blocking\(\)[\s\S]*?requestFullAccessToEventsWithCompletion/.exec(calendar);
    expect(request?.[0]).toContain('if !can_prompt(before)');
  });

  it('reports an unanswered prompt as timedOut instead of the unchanged status', () => {
    // QA's screen was locked: the prompt went unanswered for PROMPT_WAIT and
    // the request returned the unchanged writeOnly, which the card took for
    // macOS declining the upgrade. Reads the Rust source; it proves the
    // timeout branch returns the value the renderer parses as "unanswered",
    // not that the build compiles or that macOS behaves this way.
    const calendar = readFileSync(CALENDAR_RS, 'utf8');
    expect(calendar).toMatch(/pub const TIMED_OUT: &str = "timedOut";/);
    const request = /pub fn request_access_blocking\(\)[\s\S]*?\n    \}\n/.exec(calendar)?.[0] ?? '';
    expect(request).toMatch(/match rx\.recv_timeout\(PROMPT_WAIT\) \{[\s\S]*?Err\(_\) => Ok\(TIMED_OUT\.to_string\(\)\)/);
    expect(request).not.toContain('let _ = rx.recv_timeout');
  });

  it('registers every calendar command the card invokes, and defines each one', () => {
    const main = readFileSync(MAIN_RS, 'utf8');
    const calendar = readFileSync(CALENDAR_RS, 'utf8');
    expect(main).toMatch(/^mod calendar;$/m);
    for (const cmd of SHELL_COMMANDS) {
      // Unregistered, `invoke` rejects at runtime and the button does nothing.
      expect(main).toContain(`calendar::${cmd},`);
      expect(calendar).toMatch(new RegExp(`#\\[tauri::command\\]\\s*pub (async )?fn ${cmd}\\(`));
    }
  });

  it('declares the EventKit crate and every class calendar.rs calls', () => {
    // calendar.rs landed in a commit whose Cargo.toml, main.rs and Info.plist
    // did not wire it in, so a build from that commit ships a card that
    // invokes commands the shell never registered ("not available on this
    // computer"). This pins the manifest half: without the crate, or with a
    // class feature missing, calendar.rs does not compile at all.
    const cargo = readFileSync(CARGO_TOML, 'utf8');
    const eventKit = /objc2-event-kit = \{[\s\S]*?\]\s*\}/.exec(cargo)?.[0] ?? '';
    for (const feature of ['block2', 'EKCalendar', 'EKCalendarItem', 'EKEvent', 'EKEventStore', 'EKParticipant', 'EKTypes']) {
      expect(eventKit).toContain(`"${feature}"`);
    }
    const foundation = /objc2-foundation = \{[^}]*\}/.exec(cargo)?.[0] ?? '';
    for (const feature of ['NSDate', 'NSError', 'NSArray']) {
      expect(foundation).toContain(`"${feature}"`);
    }
    expect(cargo).toMatch(/^block2 = /m);
    expect(cargo).toMatch(/^chrono = /m);
  });
});
