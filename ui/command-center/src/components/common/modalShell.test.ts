/**
 * No hand-rolled modal chrome.
 *
 * This is the gate, not the fix. `DetailModal` was written as "the reusable
 * detail-view shell" and had three consumers while eleven other overlays drew
 * their own scrim — several of them byte-for-byte copies of its panel, down to
 * the `radius.lg`, the `[cardShadow, cardHighlight]` pair and the `86vh`.
 *
 * The chrome was never the point. The point is what the shell does that a copy
 * of its chrome does not: it traps Tab, it closes on Escape, it announces
 * itself as `role="dialog"` with `aria-modal` and an `aria-labelledby` title,
 * and it puts focus back on the control that opened it. Of the eleven, ONE had
 * all four. A modal that lets focus walk out behind its own scrim is operating
 * a screen the user cannot see, and every copy re-decided that from scratch.
 *
 * The rule: a full-screen scrim — `position: 'fixed'` with `inset: 0`, or
 * Tailwind's `fixed inset-0` — is written in `components/common/` and nowhere
 * else. `DetailModal` is the shell; `ConfirmDialog` (a question you answer) and
 * `FormModal` (a modal you fill in) are built on it, so all three floors are
 * one floor.
 *
 * Two lists below, and the difference between them matters:
 *  - EXEMPT is permanent. These are full-screen surfaces that are not dialogs
 *    at all, or dialogs whose shape the shell genuinely does not fit.
 *  - FRONTIER is temporary. These are modals that should be on the shell and
 *    are not yet. Each entry names why it has not moved. Deleting an entry is
 *    the migration; adding one requires a reason a reviewer would accept.
 *
 * Both are counted exactly: an entry that has stopped being true fails, and so
 * does a second scrim hiding behind an existing one.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('../..', import.meta.url));

/** Where the shells live. A shell has to draw a scrim eventually. */
const SHELL_DIR = 'components/common';

const EXEMPT: Record<string, { count: number; why: string }> = {
  'ChatApp.tsx': {
    count: 1,
    why: 'the mirrored voice orb in the detached chat window — a full-bleed '
      + 'canvas surface with no panel, no title and nothing to dismiss',
  },
  'components/splash/Splash.tsx': {
    count: 1,
    why: 'the launch splash: an app state, not a dialog. It has no owner to '
      + 'return focus to, because nothing has been opened yet',
  },
  'components/splash/BootScreen.tsx': {
    count: 1,
    why: 'the daemon-connecting boot state — a boot-phase surface rendered '
      + 'before the shell exists, not a modal',
  },
  'components/wizard/WizardShell.tsx': {
    count: 1,
    why: 'first-run setup takes the whole window on purpose — there is no '
      + 'page behind it to trap focus away from',
  },
  'components/chat/Lightbox.tsx': {
    count: 1,
    why: 'a full-bleed image on a dark mat with floating chevrons and '
      + 'arrow-key navigation between attachments — header, body and footer '
      + 'is the wrong shape for it. It needs a focus trap of its own, which '
      + 'is a fix, not a migration',
  },
  'components/voice/MeetingRecorder.tsx': {
    count: 1,
    why: 'portalled, dismissed on mousedown rather than click, and drawn on '
      + "the dropdown gradient rather than the panel surface. It already "
      + 'carries role/aria-modal/aria-label and Escape, so the shell would '
      + 'buy restyling, not a floor',
  },
};

const FRONTIER: Record<string, { count: number; why: string }> = {
  'components/settings/ConfigureProviderModal.tsx': {
    count: 1,
    why: 'already has the whole floor, hand-rolled — trap, role, aria-modal, '
      + 'labelledby, Escape, focus return. Migrating it is de-duplication '
      + '(~86 lines) rather than an a11y gain, so it queues behind the '
      + 'modals that have no floor at all',
  },
  'components/settings/AddCustomProviderModal.tsx': {
    count: 1,
    why: 'a literal copy of the trap in ConfigureProviderModal — the pair '
      + 'moves together, or the duplication survives in the other half',
  },
};

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules') continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) { sourceFiles(full, out); continue; }
    if (!/\.tsx?$/.test(entry)) continue;
    if (/\.(test|spec)\.tsx?$/.test(entry)) continue;
    out.push(full);
  }
  return out;
}

/** Comments blanked, newlines kept — the exemptions above are documented in
 *  prose that quotes the very markup it describes. */
function withoutComments(src: string): string {
  const blank = (m: string) => m.replace(/[^\n]/g, ' ');
  return src.replace(/\/\*[\s\S]*?\*\//g, blank).replace(/^[ \t]*\/\/.*$/gm, blank);
}

/** Lines that open a full-screen scrim, either spelling. */
export function scrimSites(source: string): number[] {
  const src = withoutComments(source);
  const lines: number[] = [];
  for (const m of src.matchAll(/position: 'fixed'/g)) {
    // The rest of this style object, up to its closing brace.
    const obj = src.slice(m.index, m.index + 400).split('}')[0];
    if (/\binset: 0\b/.test(obj)) lines.push(src.slice(0, m.index).split('\n').length);
  }
  for (const m of src.matchAll(/className=(?:"|\{`)[^"`]*\bfixed inset-0\b/g)) {
    lines.push(src.slice(0, m.index).split('\n').length);
  }
  return lines.sort((a, b) => a - b);
}

function scrimsByFile(): Map<string, number[]> {
  const found = new Map<string, number[]>();
  for (const file of sourceFiles(SRC)) {
    const rel = relative(SRC, file);
    if (rel.startsWith(SHELL_DIR)) continue;
    const sites = scrimSites(readFileSync(file, 'utf8'));
    if (sites.length > 0) found.set(rel, sites);
  }
  return found;
}

describe('modal shell adoption', () => {
  it('draws no full-screen scrim outside components/common', () => {
    const declared = { ...EXEMPT, ...FRONTIER };
    const offenders: string[] = [];
    for (const [rel, sites] of scrimsByFile()) {
      const allowed = declared[rel]?.count ?? 0;
      if (sites.length === allowed) continue;
      for (const line of sites) offenders.push(`${rel}:${line}`);
    }
    expect(
      offenders,
      'use DetailModal / ConfirmDialog / FormModal from components/common. A '
        + 'surface that genuinely is not a dialog goes in EXEMPT with its '
        + 'reason; a modal waiting its turn goes in FRONTIER.',
    ).toEqual([]);
  });

  it('holds every exemption and every frontier entry to its declared count', () => {
    const live = scrimsByFile();
    const wrong = Object.entries({ ...EXEMPT, ...FRONTIER })
      .map(([rel, { count }]) => {
        const found = live.get(rel)?.length ?? 0;
        return found === count ? null : `${rel}: declares ${count}, found ${found}`;
      })
      .filter(Boolean);
    expect(wrong, 'a migrated modal deletes its entry; a new scrim is not hidden behind one')
      .toEqual([]);
  });
});
