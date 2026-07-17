// @vitest-environment jsdom
//
// Exercises the REAL injected grounding logic. The test loads the exact file
// `ui/desktop/src-tauri/src/browser_grounding.js` that browser.rs injects into
// the WKWebView (via include_str!), so there is one source of truth and no
// drift between what's tested and what ships. The script only defines globals,
// so we wrap it in `new Function` and return the pieces under test; free
// references to `document` / `window` / `location` resolve to the jsdom globals.

import { describe, it, expect, beforeEach } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

interface SnapshotElement {
  ref: number;
  role: string;
  name: string;
  tag: string;
  value?: string;
}
interface Snapshot {
  url: string;
  elements: SnapshotElement[];
  truncated: boolean;
  status: string;
}
interface ActResult {
  ok: boolean;
  error?: string;
  snapshot?: Snapshot;
}

const here = dirname(fileURLToPath(import.meta.url));
const scriptPath = resolve(here, '../../../desktop/src-tauri/src/browser_grounding.js');
const src = readFileSync(scriptPath, 'utf8');

const grounding = new Function(
  src +
    '; return {' +
    ' snapshot: __permagentSnapshot,' +
    ' act: __permagentAct,' +
    ' isWebScheme: __permagentIsWebScheme,' +
    ' name: __permagentName,' +
    ' role: __permagentRole' +
    ' };',
)() as {
  snapshot: (max?: number) => Snapshot;
  act: (args: { ref: number; action: string; value?: string }, max?: number) => ActResult;
  isWebScheme: (protocol: string) => boolean;
  name: (el: Element) => string;
  role: (el: Element) => string;
};

/** Find the ref a snapshot assigned to the first element matching `predicate`. */
function refOf(snap: Snapshot, predicate: (e: SnapshotElement) => boolean): number {
  const el = snap.elements.find(predicate);
  if (!el) throw new Error('no matching element in snapshot');
  return el.ref;
}

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('web-scheme guard', () => {
  it('accepts only http(s) — the file://-and-custom-scheme rail', () => {
    expect(grounding.isWebScheme('http:')).toBe(true);
    expect(grounding.isWebScheme('https:')).toBe(true);
    expect(grounding.isWebScheme('file:')).toBe(false);
    expect(grounding.isWebScheme('about:')).toBe(false);
    expect(grounding.isWebScheme('data:')).toBe(false);
    expect(grounding.isWebScheme('permagent:')).toBe(false);
  });
});

describe('snapshot walker', () => {
  it('stamps stable refs on interactive elements in document order', () => {
    document.body.innerHTML = `
      <a href="/home">Home</a>
      <p>not interactive</p>
      <button>Search</button>
      <input type="text" aria-label="Email" />
    `;
    const snap = grounding.snapshot();
    expect(snap.status).toBe('ok');
    expect(snap.elements.map((e) => e.tag)).toEqual(['a', 'button', 'input']);
    expect(snap.elements.map((e) => e.ref)).toEqual([0, 1, 2]);
    // Refs are physically stamped for act() to find.
    expect(document.querySelector('a')!.getAttribute('data-permagent-ref')).toBe('0');
    expect(document.querySelector('input')!.getAttribute('data-permagent-ref')).toBe('2');
  });

  it('renumbers from 0 on every snapshot (never carries stale refs)', () => {
    document.body.innerHTML = `<button>One</button>`;
    grounding.snapshot();
    // Add an element before the button; a fresh snapshot must renumber.
    document.body.insertAdjacentHTML('afterbegin', `<a href="/x">X</a>`);
    const snap = grounding.snapshot();
    expect(snap.elements.map((e) => e.tag)).toEqual(['a', 'button']);
    // Exactly the current elements carry refs — no leftovers.
    expect(document.querySelectorAll('[data-permagent-ref]').length).toBe(2);
  });

  it('computes the accessible name by priority', () => {
    // aria-label wins over text
    document.body.innerHTML = `<button aria-label="Close dialog">X</button>`;
    expect(grounding.snapshot().elements[0].name).toBe('Close dialog');

    // aria-labelledby resolves referenced text
    document.body.innerHTML = `<span id="lbl">Send message</span><button aria-labelledby="lbl">go</button>`;
    expect(refOf(grounding.snapshot(), (e) => e.tag === 'button')).toBeGreaterThanOrEqual(0);
    expect(grounding.snapshot().elements.find((e) => e.tag === 'button')!.name).toBe('Send message');

    // <label> names a form control that has no text of its own
    document.body.innerHTML = `<label for="e">Email address</label><input id="e" />`;
    expect(grounding.snapshot().elements.find((e) => e.tag === 'input')!.name).toBe('Email address');

    // placeholder is used when there's no label/text
    document.body.innerHTML = `<input placeholder="Search…" />`;
    expect(grounding.snapshot().elements[0].name).toBe('Search…');

    // visible text for a plain link
    document.body.innerHTML = `<a href="/x">Read more</a>`;
    expect(grounding.snapshot().elements[0].name).toBe('Read more');

    // title is the last resort
    document.body.innerHTML = `<a href="/x" title="Tooltip only"></a>`;
    expect(grounding.snapshot().elements[0].name).toBe('Tooltip only');
  });

  it('derives roles from tag/type and honors explicit role', () => {
    document.body.innerHTML = `
      <a href="/x">l</a>
      <button>b</button>
      <input type="checkbox" aria-label="c" />
      <select aria-label="s"><option>a</option></select>
      <div role="tab" tabindex="0">t</div>
    `;
    const byTag = (t: string, i = 0) => grounding.snapshot().elements.filter((e) => e.tag === t)[i];
    expect(byTag('a').role).toBe('link');
    expect(byTag('button').role).toBe('button');
    expect(byTag('input').role).toBe('checkbox');
    expect(byTag('select').role).toBe('combobox');
    expect(grounding.snapshot().elements.find((e) => e.tag === 'div')!.role).toBe('tab');
  });

  it('reports form-control values and masks passwords', () => {
    document.body.innerHTML = `
      <input type="text" aria-label="name" value="Ada" />
      <input type="checkbox" aria-label="agree" checked />
      <input type="password" aria-label="pw" value="hunter2" />
    `;
    const snap = grounding.snapshot();
    const val = (name: string) => snap.elements.find((e) => e.name === name)!.value;
    expect(val('name')).toBe('Ada');
    expect(val('agree')).toBe('checked');
    expect(val('pw')).toBe('••••'); // never the real password
  });

  it('excludes hidden / aria-hidden / disabled elements', () => {
    document.body.innerHTML = `
      <button>visible</button>
      <button hidden>hidden-attr</button>
      <button style="display:none">display-none</button>
      <button aria-hidden="true">aria-hidden</button>
      <button disabled>disabled</button>
    `;
    const names = grounding.snapshot().elements.map((e) => e.name);
    expect(names).toEqual(['visible']);
  });

  it('caps the element count and flags truncation', () => {
    document.body.innerHTML = Array.from({ length: 10 }, (_, i) => `<button>b${i}</button>`).join('');
    const snap = grounding.snapshot(3);
    expect(snap.elements.length).toBe(3);
    expect(snap.truncated).toBe(true);
    // Uncapped: all present, not truncated.
    const full = grounding.snapshot(50);
    expect(full.elements.length).toBe(10);
    expect(full.truncated).toBe(false);
  });
});

describe('act dispatcher', () => {
  it('clicks exactly once and fires the pointer/mouse sequence', () => {
    document.body.innerHTML = `<button>Go</button>`;
    const snap = grounding.snapshot();
    const btn = document.querySelector('button')!;
    let clicks = 0;
    let mousedowns = 0;
    btn.addEventListener('click', () => (clicks += 1));
    btn.addEventListener('mousedown', () => (mousedowns += 1));

    const res = grounding.act({ ref: refOf(snap, (e) => e.tag === 'button'), action: 'click' });
    expect(res.ok).toBe(true);
    expect(clicks).toBe(1); // not double-fired
    expect(mousedowns).toBe(1);
    expect(res.snapshot?.status).toBe('ok'); // fresh snapshot handed back
  });

  it('toggles a checkbox via the native default action', () => {
    document.body.innerHTML = `<input type="checkbox" aria-label="agree" />`;
    const snap = grounding.snapshot();
    const box = document.querySelector('input')!;
    expect((box as HTMLInputElement).checked).toBe(false);
    const res = grounding.act({ ref: refOf(snap, (e) => e.tag === 'input'), action: 'click' });
    expect(res.ok).toBe(true);
    expect((box as HTMLInputElement).checked).toBe(true);
    // The fresh snapshot reflects the new state.
    expect(res.snapshot?.elements.find((e) => e.name === 'agree')?.value).toBe('checked');
  });

  it('types into an input and fires input + change', () => {
    document.body.innerHTML = `<input type="text" aria-label="Email" />`;
    const snap = grounding.snapshot();
    const input = document.querySelector('input') as HTMLInputElement;
    let inputs = 0;
    let changes = 0;
    input.addEventListener('input', () => (inputs += 1));
    input.addEventListener('change', () => (changes += 1));

    const res = grounding.act(
      { ref: refOf(snap, (e) => e.tag === 'input'), action: 'type', value: 'me@example.com' },
      50,
    );
    expect(res.ok).toBe(true);
    expect(input.value).toBe('me@example.com');
    expect(inputs).toBe(1);
    expect(changes).toBe(1);
  });

  it('selects an option by value and by visible label', () => {
    document.body.innerHTML = `
      <select aria-label="Country">
        <option value="us">United States</option>
        <option value="ca">Canada</option>
      </select>`;
    const select = document.querySelector('select') as HTMLSelectElement;
    let changes = 0;
    select.addEventListener('change', () => (changes += 1));

    // by value
    let ref = refOf(grounding.snapshot(), (e) => e.tag === 'select');
    expect(grounding.act({ ref, action: 'select', value: 'ca' }).ok).toBe(true);
    expect(select.value).toBe('ca');

    // by visible label
    ref = refOf(grounding.snapshot(), (e) => e.tag === 'select');
    expect(grounding.act({ ref, action: 'select', value: 'United States' }).ok).toBe(true);
    expect(select.value).toBe('us');
    expect(changes).toBe(2);
  });

  it('returns a structured error when no option matches', () => {
    document.body.innerHTML = `<select aria-label="s"><option value="a">A</option></select>`;
    const ref = refOf(grounding.snapshot(), (e) => e.tag === 'select');
    const res = grounding.act({ ref, action: 'select', value: 'nope' });
    expect(res.ok).toBe(false);
    expect(res.error).toMatch(/no option matched/);
    expect(res.snapshot).toBeDefined(); // still hands back a snapshot to re-ground
  });

  it('returns a structured error for a missing ref (no throw)', () => {
    document.body.innerHTML = `<button>x</button>`;
    grounding.snapshot();
    const res = grounding.act({ ref: 999, action: 'click' });
    expect(res.ok).toBe(false);
    expect(res.error).toMatch(/not found/);
  });
});
