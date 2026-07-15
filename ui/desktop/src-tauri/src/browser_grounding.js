// Permagent in-app-browser grounding logic (#649 / #622).
//
// SINGLE SOURCE OF TRUTH for the JS injected into the child WKWebView to
// (a) snapshot the page's INTERACTIVE elements as stable a11y refs and
// (b) act on a ref (click / type / select). It mirrors the Stagehand /
// Playwright-MCP discipline: ground on a stable `data-permagent-ref`, never a
// pixel; renumber refs on every snapshot; hand back a FRESH snapshot after each
// action so the caller never acts on a stale ref.
//
// This file is injected verbatim by `browser.rs` (via `include_str!`, wrapped in
// an IIFE that calls one function and JSON.stringifies the result) AND is loaded
// verbatim by the vitest+jsdom test `ui/command-center/src/lib/browserGrounding.test.ts`.
// It therefore MUST be plain ES5-ish script that only DEFINES functions — no
// `import`/`export`, no top-level side effects, no trailing call. `document`,
// `window` and `location` are referenced as globals (the page context in the
// webview; the jsdom globals under test).

// http(s) only. The agent must never drive file:// or a custom scheme into the
// webview — the same rail as the navigate route's guard in the daemon.
function __permagentIsWebScheme(protocol) {
  return protocol === 'http:' || protocol === 'https:';
}

// Interactive elements the agent can meaningfully act on. Kept in sync with the
// role/name computation below. querySelectorAll dedupes, so an element matching
// several clauses is listed once, in document (reading) order.
var __PERMAGENT_INTERACTIVE_SELECTOR = [
  'a[href]',
  'button',
  'input:not([type="hidden"])',
  'textarea',
  'select',
  'summary',
  '[contenteditable="true"]',
  '[role="button"]',
  '[role="link"]',
  '[role="checkbox"]',
  '[role="radio"]',
  '[role="switch"]',
  '[role="tab"]',
  '[role="menuitem"]',
  '[role="menuitemcheckbox"]',
  '[role="menuitemradio"]',
  '[role="option"]',
  '[role="combobox"]',
  '[role="searchbox"]',
  '[role="textbox"]',
  '[role="slider"]',
  '[role="spinbutton"]',
  '[onclick]',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

function __permagentClamp(s) {
  s = ('' + s).replace(/\s+/g, ' ').trim();
  return s.length > 120 ? s.slice(0, 117) + '...' : s;
}

// Attribute-based + (when the engine computes layout) geometry-based visibility.
// jsdom does no layout, so the geometry branch is gated on layout actually being
// available — that keeps the walker testable while still filtering offscreen /
// display:none elements in a real browser.
function __permagentVisible(el) {
  if (el.closest && el.closest('[hidden], [aria-hidden="true"]')) return false;
  if (el.matches && el.matches(':disabled')) return false;
  var inline = (el.getAttribute && el.getAttribute('style')) || '';
  if (/display\s*:\s*none/i.test(inline) || /visibility\s*:\s*hidden/i.test(inline)) return false;
  var layoutReady =
    document.body && document.body.getClientRects && document.body.getClientRects().length > 0;
  if (layoutReady) {
    if (el.getClientRects().length === 0) return false;
    var cs = window.getComputedStyle ? window.getComputedStyle(el) : null;
    if (cs && (cs.display === 'none' || cs.visibility === 'hidden')) return false;
  }
  return true;
}

// Accessible role: explicit ARIA role wins, else derived from the tag/type.
function __permagentRole(el) {
  var explicit = el.getAttribute && el.getAttribute('role');
  if (explicit && explicit.trim()) return explicit.trim();
  var tag = el.tagName.toLowerCase();
  switch (tag) {
    case 'a':
      return el.hasAttribute('href') ? 'link' : 'generic';
    case 'button':
    case 'summary':
      return 'button';
    case 'select':
      return 'combobox';
    case 'textarea':
      return 'textbox';
    case 'input': {
      var type = (el.getAttribute('type') || 'text').toLowerCase();
      if (type === 'checkbox') return 'checkbox';
      if (type === 'radio') return 'radio';
      if (type === 'submit' || type === 'button' || type === 'reset' || type === 'image') return 'button';
      if (type === 'range') return 'slider';
      return 'textbox';
    }
    default:
      return 'generic';
  }
}

// Accessible name, in priority order:
//   aria-label -> aria-labelledby -> <label> -> visible text -> placeholder
//   -> alt -> submit/button value -> title
// (<label> and the input value fallback are additions beyond the base
// aria/text/placeholder/alt/title chain — form controls have no visible text of
// their own, so their <label> or value is the only usable name.)
function __permagentName(el) {
  var t;
  if (el.getAttribute) {
    t = el.getAttribute('aria-label');
    if (t && t.trim()) return __permagentClamp(t);
    var labelledby = el.getAttribute('aria-labelledby');
    if (labelledby) {
      var joined = labelledby
        .split(/\s+/)
        .map(function (id) {
          var r = document.getElementById(id);
          return r ? r.textContent || '' : '';
        })
        .join(' ')
        .trim();
      if (joined) return __permagentClamp(joined);
    }
  }
  if (el.labels && el.labels.length) {
    var labelText = Array.prototype.map
      .call(el.labels, function (l) {
        return l.textContent || '';
      })
      .join(' ')
      .trim();
    if (labelText) return __permagentClamp(labelText);
  }
  var text = (el.textContent || '').trim();
  if (text) return __permagentClamp(text);
  if (el.getAttribute) {
    t = el.getAttribute('placeholder');
    if (t && t.trim()) return __permagentClamp(t);
    t = el.getAttribute('alt');
    if (t && t.trim()) return __permagentClamp(t);
  }
  if (el.tagName === 'INPUT') {
    var itype = (el.getAttribute('type') || 'text').toLowerCase();
    if ((itype === 'submit' || itype === 'button' || itype === 'reset') && el.value) {
      return __permagentClamp(el.value);
    }
  }
  if (el.getAttribute) {
    t = el.getAttribute('title');
    if (t && t.trim()) return __permagentClamp(t);
  }
  return '';
}

// Current value/state for form controls, so the agent can read what's filled in.
// Passwords are masked; non-form elements return undefined (field omitted).
function __permagentValue(el) {
  var tag = el.tagName.toLowerCase();
  if (tag === 'input') {
    var type = (el.getAttribute('type') || 'text').toLowerCase();
    if (type === 'checkbox' || type === 'radio') return el.checked ? 'checked' : 'unchecked';
    if (type === 'password') return el.value ? '••••' : '';
    return el.value != null ? String(el.value) : '';
  }
  if (tag === 'textarea' || tag === 'select') {
    return el.value != null ? String(el.value) : '';
  }
  return undefined;
}

// Walk the interactive elements, stamp fresh refs, return the compact list.
// Renumbers from 0 every call (clears prior refs first) so a snapshot always
// supersedes the last — refs are never carried across snapshots.
function __permagentSnapshot(maxEls) {
  var cap = typeof maxEls === 'number' && maxEls > 0 ? maxEls : 150;
  var url = location.href || '';
  if (!__permagentIsWebScheme(location.protocol)) {
    return { url: url, elements: [], truncated: false, status: 'refused_scheme' };
  }
  var stale = document.querySelectorAll('[data-permagent-ref]');
  for (var s = 0; s < stale.length; s++) stale[s].removeAttribute('data-permagent-ref');

  var nodes = document.querySelectorAll(__PERMAGENT_INTERACTIVE_SELECTOR);
  var elements = [];
  var truncated = false;
  var n = 0;
  for (var i = 0; i < nodes.length; i++) {
    var el = nodes[i];
    if (!__permagentVisible(el)) continue;
    if (n >= cap) {
      truncated = true;
      break;
    }
    el.setAttribute('data-permagent-ref', String(n));
    var entry = { ref: n, role: __permagentRole(el), name: __permagentName(el), tag: el.tagName.toLowerCase() };
    var val = __permagentValue(el);
    if (val !== undefined && val !== null) entry.value = val;
    elements.push(entry);
    n++;
  }
  return { url: url, elements: elements, truncated: truncated, status: 'ok' };
}

// Real sites listen on the pointer/mouse sequence, not only 'click'. Dispatch
// the sequence, then fire the ONE authoritative activation (native click(),
// which also performs the default action: checkbox toggle, link nav, submit).
function __permagentClick(el) {
  // No `view` — it's optional, and jsdom (and some engines) reject a cross-realm
  // window as the event view. el.click() below fires the authoritative click.
  var opts = { bubbles: true, cancelable: true };
  ['pointerdown', 'mousedown', 'pointerup', 'mouseup'].forEach(function (type) {
    var usePointer = type.indexOf('pointer') === 0 && typeof window.PointerEvent === 'function';
    var ev;
    try {
      ev = usePointer ? new window.PointerEvent(type, opts) : new window.MouseEvent(type, opts);
    } catch (e) {
      ev = new window.MouseEvent(type, opts);
    }
    el.dispatchEvent(ev);
  });
  if (typeof el.click === 'function') {
    el.click();
  } else {
    el.dispatchEvent(new window.MouseEvent('click', opts));
  }
}

// Set an input/textarea value through the NATIVE setter so React's controlled
// inputs (which shadow the value property) still observe the change.
function __permagentSetNativeValue(el, value) {
  var proto =
    el.tagName.toLowerCase() === 'textarea'
      ? window.HTMLTextAreaElement.prototype
      : window.HTMLInputElement.prototype;
  var desc = Object.getOwnPropertyDescriptor(proto, 'value');
  if (desc && desc.set) {
    desc.set.call(el, value);
  } else {
    el.value = value;
  }
}

function __permagentType(el, text) {
  if (typeof el.focus === 'function') el.focus();
  var tag = el.tagName.toLowerCase();
  if (tag === 'input' || tag === 'textarea') {
    __permagentSetNativeValue(el, text);
    el.dispatchEvent(new window.Event('input', { bubbles: true }));
    el.dispatchEvent(new window.Event('change', { bubbles: true }));
    return true;
  }
  if (el.isContentEditable) {
    el.textContent = text;
    el.dispatchEvent(new window.Event('input', { bubbles: true }));
    return true;
  }
  if ('value' in el) {
    el.value = text;
    el.dispatchEvent(new window.Event('input', { bubbles: true }));
    return true;
  }
  return false;
}

// Select an <option> by its value OR its visible label. Returns false if nothing
// matched, so the caller can report a structured error instead of silently
// leaving the control unchanged.
function __permagentSelect(el, value) {
  if (el.tagName.toLowerCase() !== 'select') {
    if ('value' in el) {
      el.value = value;
      el.dispatchEvent(new window.Event('change', { bubbles: true }));
      return true;
    }
    return false;
  }
  var options = el.options || [];
  var matched = false;
  for (var i = 0; i < options.length; i++) {
    var o = options[i];
    if (o.value === value || (o.textContent || '').trim() === value) {
      el.selectedIndex = i;
      matched = true;
      break;
    }
  }
  if (!matched) return false;
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
  el.dispatchEvent(new window.Event('change', { bubbles: true }));
  return true;
}

// Find a ref and act on it. Always returns a FRESH snapshot on success so the
// caller re-grounds on post-action refs (Playwright-MCP's key discipline).
// A missing ref is a structured error, never a throw.
function __permagentAct(args, maxEls) {
  if (!__permagentIsWebScheme(location.protocol)) {
    return { ok: false, error: 'refused_scheme: only http(s) pages can be acted on' };
  }
  var refId = args && args.ref;
  var action = args && args.action;
  var value = args && args.value;
  var el = document.querySelector('[data-permagent-ref="' + refId + '"]');
  if (!el) {
    return {
      ok: false,
      error: 'ref ' + refId + ' not found — the page may have changed; take a fresh get_page_snapshot',
    };
  }
  try {
    if (action === 'click') {
      __permagentClick(el);
    } else if (action === 'type') {
      __permagentType(el, value == null ? '' : String(value));
    } else if (action === 'select') {
      var ok = __permagentSelect(el, value == null ? '' : String(value));
      if (!ok) {
        return { ok: false, error: 'no option matched "' + value + '"', snapshot: __permagentSnapshot(maxEls) };
      }
    } else {
      return { ok: false, error: 'unknown action: ' + action };
    }
  } catch (e) {
    return { ok: false, error: 'action failed: ' + (e && e.message ? e.message : String(e)) };
  }
  return { ok: true, snapshot: __permagentSnapshot(maxEls) };
}
