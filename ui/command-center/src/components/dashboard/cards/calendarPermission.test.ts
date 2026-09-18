/**
 * The macOS keys without which a Calendar permission prompt can never appear.
 *
 * The daemon reads today's events by sending Apple events to Calendar.app
 * (`osascript` + `tell application "Calendar"`, in goose-server's
 * `dashboard_cards.rs`). That is Automation, not EventKit — nothing in the
 * tree links EventKit at all — so the keys macOS consults are
 * NSAppleEventsUsageDescription and the hardened-runtime entitlement
 * `com.apple.security.automation.apple-events`. NSCalendarsUsageDescription
 * would be decoration.
 *
 * With the usage description absent, macOS refuses the event with -1743 and
 * shows no prompt at all, which is exactly the state the user hit: a card
 * telling them to grant access, and no prompt to grant it in.
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
});
