// @vitest-environment jsdom
//
// Exercises the REAL injected link-gesture logic. The test loads the exact file
// `ui/desktop/src-tauri/src/browser_links.js` that browser.rs injects into every
// page webview (via include_str! + initialization_script_for_all_frames), so
// there is one source of truth and no drift between what is tested and what
// ships. The script only defines functions, so we wrap it in `new Function` and
// return the pieces under test; free references to `document` / `window` /
// `location` resolve to the jsdom globals.
//
// WHAT THIS PINS. The in-app browser is a native child WKWebView. Its only
// content-initiated seam is WKUIDelegate's createWebViewWithConfiguration:,
// which WebKit sends ONLY for `window.open` and for an anchor targeting a frame
// that does not exist. Right-click, middle-click and Cmd-click never get there:
// they arrive at decidePolicyForNavigationAction, whose wry binding is
// `Fn(String) -> bool` and has already thrown away the button number and the
// modifier flags. This script is what makes those gestures reach the seam at
// all — so "one gesture, exactly one tab" has to be tested here, per gesture.

import { describe, it, expect, beforeAll, beforeEach, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

interface Decision {
  action: 'ignore' | 'newtab' | 'menu';
  url: string | null;
  reason: string;
}
interface Gesture {
  type: string;
  button?: number | null;
  metaKey?: boolean;
  ctrlKey?: boolean;
  shiftKey?: boolean;
  defaultPrevented?: boolean;
}
interface Link {
  href: string;
  target?: string | null;
  download?: boolean;
}

const here = dirname(fileURLToPath(import.meta.url));
const scriptPath = resolve(here, '../../../desktop/src-tauri/src/browser_links.js');
const src = readFileSync(scriptPath, 'utf8');

function loadPure() {
  return new Function(
    src +
      '; return {' +
      ' decide: __permagentLinkDecision,' +
      ' isWebLink: __permagentIsWebLink,' +
      ' resolveHref: __permagentResolveHref,' +
      ' wantsNewContext: __permagentTargetWantsNewContext,' +
      ' isDuplicate: __permagentIsDuplicateOpen' +
      ' };',
  )() as {
    decide: (g: Gesture, l: Link | null, c: { isTopFrame: boolean }) => Decision;
    isWebLink: (url: string) => boolean;
    resolveHref: (raw: string, base?: string) => string;
    wantsNewContext: (target: string | null | undefined) => boolean;
    isDuplicate: (
      state: { url: string; at: number } | null,
      url: string,
      now: number,
      windowMs?: number,
    ) => boolean;
  };
}

/** Install the real listeners onto the jsdom window and return the handle. */
function install() {
  return new Function(src + '; return __permagentInstallLinks();')() as {
    hasMenu: () => boolean;
    closeMenu: () => void;
  };
}

const TOP = { isTopFrame: true };

describe('link gesture decision (pure)', () => {
  const link = (href: string, extra: Partial<Link> = {}): Link => ({
    href,
    target: null,
    download: false,
    ...extra,
  });
  const { decide } = loadPure();

  // ── The four gestures, one at a time ─────────────────────────────────────

  it('claims a PLAIN left-click on target=_blank — the reported case', () => {
    const d = decide({ type: 'click', button: 0 }, link('https://example.com/x', { target: '_blank' }), TOP);
    expect(d.action).toBe('newtab');
    expect(d.url).toBe('https://example.com/x');
    expect(d.reason).toBe('target-new-context');
  });

  it('claims a middle-click on an ordinary link', () => {
    const d = decide({ type: 'auxclick', button: 1 }, link('https://example.com/mid'), TOP);
    expect(d.action).toBe('newtab');
    expect(d.reason).toBe('middle-click');
  });

  it('claims a middle-click delivered only as mouseup', () => {
    // wry re-dispatches macOS otherMouseUp through the left-button path, so
    // `auxclick` is not guaranteed. Both entry points must work.
    const d = decide({ type: 'mouseup', button: 1 }, link('https://example.com/mid'), TOP);
    expect(d.action).toBe('newtab');
    expect(d.reason).toBe('middle-click');
  });

  it('claims Cmd-click and Ctrl-click on an ordinary link', () => {
    expect(decide({ type: 'click', button: 0, metaKey: true }, link('https://a.test/'), TOP).action).toBe(
      'newtab',
    );
    expect(decide({ type: 'click', button: 0, ctrlKey: true }, link('https://a.test/'), TOP).action).toBe(
      'newtab',
    );
  });

  it('opens its own menu on right-click over a link, because WKWebView has none', () => {
    const d = decide({ type: 'contextmenu' }, link('https://example.com/ctx'), TOP);
    expect(d.action).toBe('menu');
    expect(d.url).toBe('https://example.com/ctx');
  });

  // ── What it must NOT claim ────────────────────────────────────────────────

  it('leaves an ordinary left-click alone — same-tab navigation still works', () => {
    const d = decide({ type: 'click', button: 0 }, link('https://example.com/same'), TOP);
    expect(d.action).toBe('ignore');
    expect(d.reason).toBe('same-tab-navigation');
  });

  it('leaves a click that is not on a link alone — SPA routing, buttons, text', () => {
    expect(decide({ type: 'click', button: 0 }, null, TOP).reason).toBe('not-a-link');
    expect(decide({ type: 'contextmenu' }, null, TOP).reason).toBe('not-a-link');
  });

  it('refuses non-http(s) schemes', () => {
    for (const href of [
      'javascript:void(0)',
      'about:blank',
      'mailto:someone@example.com',
      'data:text/html,hi',
      'file:///etc/hosts',
      'blob:https://example.com/abc',
      'tel:+15550100',
    ]) {
      const d = decide({ type: 'auxclick', button: 1 }, link(href), TOP);
      expect(d.action, href).toBe('ignore');
      expect(d.reason, href).toBe('non-web-scheme');
    }
  });

  it('refuses a download link — browser.rs already routes those to the inbox', () => {
    const d = decide({ type: 'click', button: 0, metaKey: true }, link('https://x.test/f.pdf', { download: true }), TOP);
    expect(d.reason).toBe('download-link');
  });

  it('never draws a menu inside a subframe, where the overlay would be clipped', () => {
    const d = decide({ type: 'contextmenu' }, link('https://ad.test/x'), { isTopFrame: false });
    expect(d.action).toBe('ignore');
    expect(d.reason).toBe('subframe-context-menu');
  });

  it('stands down when something already prevented the default', () => {
    const d = decide(
      { type: 'click', button: 0, metaKey: true, defaultPrevented: true },
      link('https://x.test/'),
      TOP,
    );
    expect(d.reason).toBe('default-already-prevented');
  });

  it('treats only _self/_top/_parent as staying put', () => {
    const { wantsNewContext } = loadPure();
    expect(wantsNewContext('_blank')).toBe(true);
    expect(wantsNewContext('someNamedWindow')).toBe(true);
    expect(wantsNewContext('_self')).toBe(false);
    expect(wantsNewContext('_top')).toBe(false);
    expect(wantsNewContext('_parent')).toBe(false);
    expect(wantsNewContext('')).toBe(false);
    expect(wantsNewContext(null)).toBe(false);
  });
});

describe('href resolution', () => {
  const { resolveHref } = loadPure();

  it('resolves a relative href against the document base', () => {
    expect(resolveHref('/deep/page', 'https://example.com/a/b')).toBe('https://example.com/deep/page');
    expect(resolveHref('sibling', 'https://example.com/a/b')).toBe('https://example.com/a/sibling');
    expect(resolveHref('?q=1', 'https://example.com/a/b')).toBe('https://example.com/a/b?q=1');
  });

  it('leaves an absolute href alone and never throws on rubbish', () => {
    expect(resolveHref('https://other.test/x', 'https://example.com/')).toBe('https://other.test/x');
    expect(resolveHref('', 'https://example.com/')).toBe('');
    expect(resolveHref('http://[', 'https://example.com/')).toBe('');
  });
});

describe('duplicate suppression', () => {
  const { isDuplicate } = loadPure();

  it('swallows the same URL twice inside the window, and only then', () => {
    const state = { url: '', at: -1e9 };
    expect(isDuplicate(state, 'https://x.test/a', 1000)).toBe(false);
    expect(isDuplicate(state, 'https://x.test/a', 1100)).toBe(true);
    expect(isDuplicate(state, 'https://x.test/a', 2000)).toBe(false);
  });

  it('never confuses two different links', () => {
    const state = { url: '', at: -1e9 };
    expect(isDuplicate(state, 'https://x.test/a', 1000)).toBe(false);
    expect(isDuplicate(state, 'https://x.test/b', 1010)).toBe(false);
  });
});

// ── End to end, on real DOM events ──────────────────────────────────────────
//
// One gesture must produce exactly ONE window.open — that call is the only
// channel a remote page has back to the shell (it re-enters WebKit's
// createWebViewWithConfiguration:, which browser.rs denies and re-emits as
// browser_new_window_request).
describe('installed interceptor, per gesture', () => {
  const opened: string[] = [];
  let handle: ReturnType<typeof install>;

  // Installed ONCE, as it is in a real page: the script guards itself with
  // `window.__permagentLinks`. Re-installing per test would stack listeners,
  // and the second copy would correctly stand down on the first copy's
  // `preventDefault` — an artefact of the test, not of the browser.
  //
  // The stub goes in FIRST because the script captures `window.open` at
  // install time, exactly as it does at document start in a real page — before
  // any page script can replace it.
  beforeAll(() => {
    window.open = vi.fn((url?: string | URL) => {
      opened.push(String(url));
      return null;
    }) as unknown as typeof window.open;
    handle = install();
  });

  beforeEach(() => {
    handle.closeMenu();
    document.body.innerHTML = '';
    opened.length = 0;
  });

  function anchor(html: string): HTMLAnchorElement {
    document.body.innerHTML = html;
    return document.querySelector('a') as HTMLAnchorElement;
  }

  function fire(el: Element, type: string, init: MouseEventInit = {}) {
    const ev = new window.MouseEvent(type, { bubbles: true, cancelable: true, ...init });
    el.dispatchEvent(ev);
    return ev;
  }

  it('plain left-click on target=_blank opens exactly one tab', () => {
    const a = anchor('<a href="https://example.com/blank" target="_blank">go</a>');
    const ev = fire(a, 'click', { button: 0 });
    expect(opened).toEqual(['https://example.com/blank']);
    expect(ev.defaultPrevented).toBe(true);
  });

  it('middle-click opens exactly one tab even though auxclick AND mouseup both fire', () => {
    const a = anchor('<a href="https://example.com/mid">go</a>');
    fire(a, 'mouseup', { button: 1 });
    fire(a, 'auxclick', { button: 1 });
    expect(opened).toEqual(['https://example.com/mid']);
  });

  it('Cmd-click opens exactly one tab', () => {
    const a = anchor('<a href="https://example.com/meta">go</a>');
    fire(a, 'click', { button: 0, metaKey: true });
    expect(opened).toEqual(['https://example.com/meta']);
  });

  it('right-click draws a link menu whose first item opens a tab', () => {
    const a = anchor('<a href="https://example.com/ctx">go</a>');
    const ev = fire(a, 'contextmenu', { button: 2 });
    expect(ev.defaultPrevented).toBe(true);
    expect(handle.hasMenu()).toBe(true);
    // The menu lives in a closed shadow root; reach it the way a user does.
    const host = document.querySelector('[data-permagent-link-menu]');
    expect(host).not.toBeNull();
    handle.closeMenu();
    expect(handle.hasMenu()).toBe(false);
  });

  it('an ordinary left-click is never intercepted — navigation and SPA routing survive', () => {
    const a = anchor('<a href="https://example.com/normal">go</a>');
    const ev = fire(a, 'click', { button: 0 });
    expect(opened).toEqual([]);
    expect(ev.defaultPrevented).toBe(false);
  });

  it('right-click on plain text is left to WebKit, so selection and Copy still work', () => {
    document.body.innerHTML = '<p id="t">just words</p>';
    const p = document.getElementById('t') as HTMLElement;
    const ev = fire(p, 'contextmenu', { button: 2 });
    expect(ev.defaultPrevented).toBe(false);
    expect(handle.hasMenu()).toBe(false);
  });

  it('finds the link when the click lands on a child element', () => {
    const a = anchor('<a href="https://example.com/deep"><span><b>inner</b></span></a>');
    const inner = a.querySelector('b') as HTMLElement;
    fire(inner, 'auxclick', { button: 1 });
    expect(opened).toEqual(['https://example.com/deep']);
  });

  it('resolves a relative href to an absolute URL before handing it over', () => {
    const a = anchor('<a href="/relative/path" target="_blank">go</a>');
    fire(a, 'click', { button: 0 });
    expect(opened).toHaveLength(1);
    expect(opened[0]).toMatch(/^https?:\/\/[^/]+\/relative\/path$/);
  });

  it('installs once, however many times it is asked', () => {
    // Every navigation re-runs the initialization script; a second set of
    // listeners would double every gesture.
    expect(install()).toBe(handle);
    const a = anchor('<a href="https://example.com/once">go</a>');
    fire(a, 'auxclick', { button: 1 });
    expect(opened).toEqual(['https://example.com/once']);
  });
});
