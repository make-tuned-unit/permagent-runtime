/**
 * No hand-rolled `<button>` in a migrated directory.
 *
 * This is the gate, not the fix. `Button` was documented as "the app's one
 * button primitive" and still lost 491 to 1, because nothing anywhere said no.
 * An inline `style` object cannot express `:hover` or `:active` at all, so
 * every one of those 491 was pressable with no acknowledgement and disable-able
 * with no visible difference — and each one that was fixed by hand was fixed
 * differently, which is how the app ended up with three separate button
 * contracts.
 *
 * The rule, inside a gated directory: a `<button>` element is allowed only when
 * it wears `.pa-btn` — the shared interaction rules — and has a reason not to
 * be a `Button`. There are exactly two such reasons, and both are about
 * semantics the primitive would flatten:
 *
 *   1. DISCLOSURE TOGGLES (`aria-expanded` + `aria-controls`). Precedent:
 *      commit 4883d2f2, Finance's pick row. There is nothing to await, so the
 *      pending floor and the success tick are the wrong signals for it, and
 *      `Button` would displace the aria pairing that actually describes what
 *      pressing it does. It still gets hover, press and focus, through the
 *      class rather than through a pair of mouse handlers.
 *   2. ROLE-BEARING CONTROLS (`role="tab"`, `role="switch"`, `role="menuitem"`,
 *      `role="option"`). Same argument: the role is the description, and
 *      `Button` renders a plain button.
 *   3. COMPOSITE CONTROLS (`.pa-btn--composite`), whose children have to be the
 *      button's own flex children — a `flex: 1` name that truncates, meta
 *      pushed to the right edge. `Button` folds children into a single
 *      `pa-btn__label` span, and inside an inline box `overflow: hidden` does
 *      nothing, so the distribution collapses. The modifier is a declaration
 *      that the layout is the reason; `index.css` defines it.
 *
 * `components/common/` is exempt wholesale — it is where primitives are
 * written, and a primitive has to render an element eventually.
 *
 * TO EXTEND: add your directory to `GATED` in the same commit that migrates it.
 * That is the whole protocol. A directory not listed here is not yet migrated,
 * which is a statement about the frontier rather than a licence.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('../..', import.meta.url));

/** Directories whose controls are on the primitive. Grows one commit at a time. */
const GATED = [
  'components/automate',
  'components/awareness',
  'components/brain',
  'components/browser',
  'components/build',
  'components/chat',
  'components/dashboard',
  'components/finance',
  'components/goals',
  'components/grow',
  'components/inbox',
  'components/inspection',
  'components/notifications',
  'components/people',
  'components/projects',
  'components/sessions',
  'components/settings',
  'components/sidebar',
  'components/skills',
  'components/terminal',
  'components/tool-results',
  'components/trace',
  'components/version',
  'components/voice',
  'components/wizard',
  'components/workspaces',
  'components/world',
];

/**
 * Controls that stay hand-rolled for a reason the three classes above do not
 * cover — almost always a whole card that happens to be a button, where
 * `.pa-btn`'s own box would relayout it. Each entry names a COUNT as well as a
 * reason, and the count is checked exactly: an entry that has stopped being
 * true fails, and so does a new raw button hiding behind an existing one.
 */
const STANDING_EXCEPTIONS: Record<string, { count: number; why: string }> = {
  'components/awareness/CitationMarker.tsx': {
    count: 2,
    why: 'the whole citation row is the control: a meta line stacked above a summary',
  },
  'components/brain/BrainView.tsx': {
    count: 1,
    why: "the selection panel's stat blocks are two-line and half of them are "
      + 'permanently disabled by design, which the primitive would grey out',
  },
  'components/chat/ChatLauncher.tsx': {
    count: 1,
    why: 'the launcher pill lifts 2px on hover and settles on press — a transform '
      + 'the primitive has no property for, and an inline one would beat its own',
  },
  'components/dashboard/AddCardPicker.tsx': {
    count: 1,
    why: 'the whole add-a-card row is the control: an icon tile beside a two-line '
      + 'name and description',
  },
  'components/dashboard/cards/GrowthResultsCard.tsx': {
    count: 1,
    why: 'the whole project row is the control: truncating name, stats line and a '
      + 'fixed-width sparkline',
  },
  'components/dashboard/cards/TodosCard.tsx': {
    count: 1,
    why: 'the whole todo row is the control: a two-line title and provenance block',
  },
  'components/notifications/NotificationHost.tsx': {
    count: 1,
    why: 'the tray row is the control: title, body and timestamp stacked by '
      + 'the button itself (R5: the toast moved to its own file, Toast.tsx, '
      + 'below)',
  },
  'components/notifications/Toast.tsx': {
    count: 1,
    why: 'the toast is the control: title and body stacked by the button '
      + 'itself, plus its own spring-in/out and dismiss timer — split out of '
      + 'NotificationHost.tsx (R5) so that behaviour is testable on its own',
  },
  'components/people/PersonFace.tsx': {
    count: 1,
    why: 'the graph-node avatar draws a 2px ring and computes opacity, shadow and '
      + 'transform inline, so an inline transform would beat the press give — and '
      + 'the same style object is reused by its non-interactive twin',
  },
  'components/projects/MemoriesPanel.tsx': {
    count: 1,
    why: 'the whole memory row is the control: a clamped two-line description above '
      + 'a meta row, top-aligned',
  },
  'components/settings/SettingsView.tsx': {
    count: 1,
    why: 'the trust-level options are two-line cards: a title row with a badge '
      + 'above a description',
  },
  'components/settings/agents/AgentsPanel.tsx': {
    count: 2,
    why: 'the worker and persona rows are whole cards — a portrait, a status row '
      + 'and a description — that open a detail page',
  },
  'components/skills/SkillCard.tsx': {
    count: 1,
    why: 'the whole skill card is the control, four nested rows of its own layout',
  },
  'components/wizard/MomentCalibration.tsx': {
    count: 1,
    why: 'the persona preset is a two-line card in a two-column grid',
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

/** Comments blanked, newlines kept so line numbers still mean something. The
 *  exemptions are documented in prose next to the code they cover, and that
 *  prose quotes the markup it is describing — a gate that reads its own
 *  documentation as a violation teaches people to stop writing it. */
function withoutComments(src: string): string {
  const blank = (m: string) => m.replace(/[^\n]/g, ' ');
  return src.replace(/\/\*[\s\S]*?\*\//g, blank).replace(/^[ \t]*\/\/.*$/gm, blank);
}

/** Every `<button …>` opening tag, whole, with the line it starts on. Braces
 *  and quotes are tracked so the `>` of an arrow function inside an attribute
 *  does not look like the end of the tag. */
export function buttonOpeningTags(source: string): { line: number; tag: string }[] {
  const src = withoutComments(source);
  const found: { line: number; tag: string }[] = [];
  const re = /<button(?=[\s/>])/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    let i = m.index + '<button'.length;
    let depth = 0;
    let quote: string | null = null;
    for (; i < src.length; i += 1) {
      const c = src[i];
      if (quote) {
        if (c === quote && src[i - 1] !== '\\') quote = null;
        continue;
      }
      if (c === '"' || c === "'" || c === '`') { quote = c; continue; }
      if (c === '{') depth += 1;
      else if (c === '}') depth -= 1;
      else if (c === '>' && depth === 0) break;
    }
    found.push({
      line: src.slice(0, m.index).split('\n').length,
      tag: src.slice(m.index, i + 1),
    });
  }
  return found;
}

/** Wears the shared interaction rules, and has a semantic `Button` would lose. */
function isPermittedRawButton(tag: string): boolean {
  const wearsClass = /className=(?:"[^"]*\bpa-btn\b|'[^']*\bpa-btn\b|\{[^}]*pa-btn)/.test(tag);
  if (!wearsClass) return false;
  return /\baria-expanded[=\s]/.test(tag)
    || /\brole=/.test(tag)
    || /\bpa-btn--composite\b/.test(tag);
}

function offendersIn(dirs: string[]): string[] {
  const out: string[] = [];
  for (const dir of dirs) {
    for (const file of sourceFiles(join(SRC, dir))) {
      const rel = file.slice(SRC.length);
      const allowed = STANDING_EXCEPTIONS[rel]?.count ?? 0;
      const src = readFileSync(file, 'utf8');
      const raw = buttonOpeningTags(src).filter(({ tag }) => !isPermittedRawButton(tag));
      if (raw.length === allowed) continue;
      for (const { line } of raw) out.push(`${rel}:${line}`);
    }
  }
  return out;
}

describe('button primitive adoption', () => {
  it('leaves no hand-rolled button in a migrated directory', () => {
    expect(
      offendersIn(GATED),
      'use <Button> from components/common/Button. A disclosure toggle, a '
        + 'role-bearing control or a .pa-btn--composite may stay a <button>, '
        + 'but must carry className="pa-btn".',
    ).toEqual([]);
  });

  it('holds every standing exception to the count it declares', () => {
    const wrong = Object.entries(STANDING_EXCEPTIONS)
      .map(([rel, { count }]) => {
        const src = readFileSync(join(SRC, rel), 'utf8');
        const raw = buttonOpeningTags(src).filter(({ tag }) => !isPermittedRawButton(tag)).length;
        return raw === count ? null : `${rel}: declares ${count}, found ${raw}`;
      })
      .filter(Boolean);
    expect(wrong, 'a spent exception is deleted; a new raw button is migrated').toEqual([]);
  });

  it('names only directories that exist', () => {
    const missing = GATED.filter(d => {
      try { return !statSync(join(SRC, d)).isDirectory(); } catch { return true; }
    });
    expect(missing).toEqual([]);
  });
});
