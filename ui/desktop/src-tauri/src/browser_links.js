// Permagent in-app-browser link gestures (#240 / #709 / #973, and the 2026-08-18
// report "I still cant click a link in one tab and have it open in a new tab").
//
// WHY THIS FILE EXISTS
//
// The whole "open in a new tab" feature used to rest on ONE native callback:
// wry bridges WKUIDelegate's
// `webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:`
// to `WebviewBuilder::on_new_window` (see browser.rs). WebKit only sends that
// message when the PAGE CONTENT asks for a new frame — `window.open(...)` or a
// click on an anchor whose `target` names a frame that does not exist
// (`_blank`). Every MOUSE way a person asks for a new tab —
//
//   * right-click -> "Open Link in New Tab"
//   * middle-click (button 1)
//   * Cmd-click / Ctrl-click
//
// — is not a content request for a new frame. Those arrive at
// `decidePolicyForNavigationAction`, whose wry binding is
// `Fn(String) -> bool`: the button number and the modifier flags are gone
// before Rust can see them. And a bare WKWebView has no "Open Link in New Tab"
// menu item at all — that item is Safari's, not WebKit's. So the mouse-only
// user had no working gesture and the click looked like it did nothing.
//
// This script closes that gap IN THE PAGE. It claims the gestures WebKit will
// not route to the UI delegate and re-expresses them as `window.open(url,
// '_blank')`, which WebKit DOES route there. Everything therefore converges on
// the one hook the Rust guard test pins and the one tab-opening path in
// Browser.tsx — no second channel to rot, no second way to open a tab.
//
// It also draws its own link context menu (in a shadow root, top frame only),
// because WKWebView's default menu offers no "Open Link in New Tab" and the
// remote page has no Tauri bridge to ask the React shell for one.
//
// SHAPE CONTRACT — this file is injected verbatim by `browser.rs` (via
// `include_str!`, wrapped in an IIFE that calls `__permagentInstallLinks()`)
// AND loaded verbatim by the vitest+jsdom test
// `ui/command-center/src/lib/browserLinks.test.ts`. It MUST therefore be plain
// ES5-ish script that only DEFINES things — no `import`/`export`, no top-level
// side effects, no trailing call. `document`, `window` and `location` are
// referenced as globals (the page in the webview; the jsdom globals in test).

// Only ever hand an http(s) URL to the shell. Same rail as the grounding
// script's scheme guard: `javascript:`, `about:`, `data:`, `blob:`, `file:`,
// `mailto:` and friends must fall through to the page / WebKit untouched.
function __permagentIsWebLink(url) {
  return /^https?:\/\//i.test(String(url || ''));
}

// Absolute URL for an anchor's href. The DOM's `.href` property is already
// resolved, but SVG anchors and detached nodes are not, so a raw attribute is
// resolved against the document base. Returns '' when it cannot be resolved —
// never throws, because this runs inside somebody else's page.
function __permagentResolveHref(rawHref, baseHref) {
  var raw = String(rawHref == null ? '' : rawHref).trim();
  if (!raw) return '';
  try {
    return new URL(raw, baseHref || undefined).href;
  } catch (e) {
    return '';
  }
}

// A `target` that asks for a NEW browsing context.
//
// '' / _self / _top / _parent all stay in the current one, and `_blank` always
// wants a new one. A NAMED target is the case that needs care: it means "put
// the result in the context called this", and that is only a new-tab request
// when no such context EXISTS. If the page carries `<iframe name="preview">`
// then `<a target="preview">` is an ordinary in-page navigation, and a real
// browser loads it into that frame.
//
// Getting this wrong is not cosmetic. Claiming such a click cancels the real
// navigation (`handle` calls `preventDefault`) and re-expresses it as
// `window.open`, which browser.rs DENIES and re-routes to a brand new tab —
// so the page's own frame never loads and the navigation is torn out of the
// browsing context that started it. Sign-in and payment flows that drive a
// named frame are exactly the kind that cannot survive that, because the
// state they left behind (`window.name`, the frame's document, an opener
// relationship) is in the context the navigation just left.
//
// `frameNames` is the list of frame names present in the document, supplied by
// `context()`; absent or empty it means "none", which is the pre-existing
// behaviour for every page that has no named frames.
function __permagentTargetWantsNewContext(target, frameNames) {
  var t = String(target == null ? '' : target).trim().toLowerCase();
  if (!t) return false;
  if (t === '_self' || t === '_top' || t === '_parent') return false;
  if (t === '_blank') return true;
  return !__permagentTargetNamesAFrameInPage(t, frameNames);
}

// True when `target` names a frame that already exists in this document.
function __permagentTargetNamesAFrameInPage(target, frameNames) {
  var t = String(target == null ? '' : target).trim().toLowerCase();
  if (!t) return false;
  var names = frameNames || [];
  for (var i = 0; i < names.length; i++) {
    if (String(names[i] == null ? '' : names[i]).trim().toLowerCase() === t) return true;
  }
  return false;
}

// ── The decision, as a pure function ────────────────────────────────────────
//
// `gesture` is a plain description of the DOM event
//   { type, button, metaKey, ctrlKey, shiftKey, defaultPrevented }
// `link` is the resolved anchor under the pointer, or null
//   { href, target, download }
// `ctx` is the frame's situation
//   { isTopFrame, frameNames }
//
// Returns { action: 'ignore' | 'newtab' | 'menu', url, reason }. `reason` is
// always populated, including for 'ignore' — a dropped gesture must leave a
// trace, which is exactly what the three previous regressions did not.
function __permagentLinkDecision(gesture, link, ctx) {
  var g = gesture || {};
  var c = ctx || {};
  var type = String(g.type || '');

  if (g.defaultPrevented) {
    return { action: 'ignore', url: null, reason: 'default-already-prevented' };
  }
  if (!link || !link.href) {
    return { action: 'ignore', url: null, reason: 'not-a-link' };
  }
  if (!__permagentIsWebLink(link.href)) {
    return { action: 'ignore', url: null, reason: 'non-web-scheme' };
  }
  // `download` means "save it", and browser.rs already redirects downloads into
  // the inbox. Opening a tab as well would both save AND navigate.
  if (link.download) {
    return { action: 'ignore', url: null, reason: 'download-link' };
  }

  if (type === 'contextmenu') {
    // Only the top frame draws the menu: a shadow-root overlay inside a
    // cross-origin ad iframe would be clipped to that iframe's box. Subframes
    // fall through to WebKit's own menu.
    if (!c.isTopFrame) {
      return { action: 'ignore', url: null, reason: 'subframe-context-menu' };
    }
    return { action: 'menu', url: link.href, reason: 'link-context-menu' };
  }

  // Middle-click. WebKit delivers it as `auxclick` with button 1; some pages
  // and some WebKit builds only produce `mouseup`, so both are accepted.
  if ((type === 'auxclick' || type === 'mouseup') && g.button === 1) {
    return { action: 'newtab', url: link.href, reason: 'middle-click' };
  }

  if (type === 'click' && (g.button === 0 || g.button == null)) {
    // Cmd-click (macOS) / Ctrl-click (elsewhere). Lowest-priority gesture, but
    // free once the plumbing exists. NB on macOS Ctrl+left-click is delivered
    // as `contextmenu` and produces no `click`, so this cannot double-fire.
    if (g.metaKey || g.ctrlKey) {
      return { action: 'newtab', url: link.href, reason: 'modifier-click' };
    }
    // A PLAIN left-click on target="_blank" — the common case on real sites,
    // and the one the owner reports. WebKit would route this to the UI
    // delegate on its own; we claim it anyway and cancel the default, so the
    // gesture takes the SAME path as every other one and can only produce one
    // tab. Claiming it is also what makes the behaviour independent of
    // WebKit's willingness to consult the delegate for this particular action.
    if (__permagentTargetWantsNewContext(link.target, c.frameNames)) {
      return { action: 'newtab', url: link.href, reason: 'target-new-context' };
    }
    // Named a frame this document owns: the page navigates it itself. Reported
    // as its own reason so a dropped gesture is never a silent one.
    if (__permagentTargetNamesAFrameInPage(link.target, c.frameNames)) {
      return { action: 'ignore', url: null, reason: 'targets-a-frame-in-the-page' };
    }
    // Everything else is ordinary browsing: same-tab navigation, SPA routing,
    // a page's own click handler. Never claimed.
    return { action: 'ignore', url: null, reason: 'same-tab-navigation' };
  }

  return { action: 'ignore', url: null, reason: 'not-a-new-tab-gesture' };
}

// ── Duplicate suppression ───────────────────────────────────────────────────
//
// Belt and braces means two things can fire for one click (ours, and WebKit's
// own new-frame path if a `preventDefault` is ever lost). One click must still
// be one tab. `state` is `{ url, at }` or null and is MUTATED in place, so the
// caller can keep it in a closure and the test can hand in a literal.
var __PERMAGENT_DUPLICATE_WINDOW_MS = 500;

function __permagentIsDuplicateOpen(state, url, now, windowMs) {
  var span = typeof windowMs === 'number' ? windowMs : __PERMAGENT_DUPLICATE_WINDOW_MS;
  if (state && state.url === url && now - state.at < span) return true;
  if (state) {
    state.url = url;
    state.at = now;
  }
  return false;
}

// ── Installation ────────────────────────────────────────────────────────────

// Nearest ancestor anchor with an href, following `composedPath` so a link
// inside a shadow root (web-component sites) is still found.
function __permagentFindAnchor(ev) {
  var path = null;
  try {
    path = typeof ev.composedPath === 'function' ? ev.composedPath() : null;
  } catch (e) {
    path = null;
  }
  var node;
  if (path && path.length) {
    for (var i = 0; i < path.length; i++) {
      node = path[i];
      if (node && node.nodeType === 1 && node.tagName && node.tagName.toLowerCase() === 'a') {
        if (node.getAttribute && node.getAttribute('href') != null) return node;
      }
    }
  }
  node = ev.target;
  while (node && node.nodeType === 1) {
    if (node.tagName && node.tagName.toLowerCase() === 'a' && node.getAttribute('href') != null) {
      return node;
    }
    node = node.parentNode;
  }
  return null;
}

// The `link` half of a decision, read off a DOM anchor.
function __permagentReadLink(anchor) {
  if (!anchor) return null;
  var base = '';
  try {
    base = (anchor.ownerDocument && anchor.ownerDocument.baseURI) || String(location.href);
  } catch (e) {
    base = '';
  }
  var raw = anchor.getAttribute ? anchor.getAttribute('href') : '';
  // `.href` is the resolved form for HTML anchors; SVG anchors have no such
  // property, hence the attribute + base fallback.
  var href = typeof anchor.href === 'string' && anchor.href ? anchor.href : '';
  if (!__permagentIsWebLink(href)) href = __permagentResolveHref(raw, base);
  return {
    href: href,
    target: anchor.getAttribute ? anchor.getAttribute('target') : '',
    download: !!(anchor.hasAttribute && anchor.hasAttribute('download')),
  };
}

function __permagentInstallLinks() {
  if (window.__permagentLinks) return window.__permagentLinks;

  // Captured at document start, BEFORE any page script runs. Plenty of sites
  // replace `window.open` with their own (analytics wrappers, popup managers,
  // consent shims); calling theirs would hand the URL to code that has no idea
  // what to do with it, after we have already cancelled the real navigation.
  var nativeOpen = window.open;

  var lastOpen = { url: '', at: -1e9 };
  var menuHost = null;

  function log(message, detail) {
    // A dropped gesture must leave a trace. This is the page-side half of the
    // logging the Rust emitter and Browser.tsx now do.
    try {
      console.debug('[permagent] links: ' + message, detail === undefined ? '' : detail);
    } catch (e) {
      /* a page may have replaced console */
    }
  }

  function openInNewTab(url, reason) {
    var now = Date.now();
    if (__permagentIsDuplicateOpen(lastOpen, url, now)) {
      log('suppressed duplicate open', url);
      return false;
    }
    log('open in new tab (' + reason + ')', url);
    try {
      // The ONLY channel out of the page: a remote page has no Tauri bridge
      // (see create_browser_webview), so this is how the URL gets to the
      // shell. WebKit routes it to createWebViewWithConfiguration:, wry to
      // on_new_window, browser.rs to `browser_new_window_request`, Browser.tsx
      // to a tab. It returns null here because Rust denies the native window
      // on purpose — that is success, not failure, so it is never treated as
      // one.
      nativeOpen.call(window, url, '_blank');
    } catch (e) {
      log('window.open threw', String(e));
      return false;
    }
    return true;
  }

  function closeMenu() {
    if (!menuHost) return;
    try {
      if (menuHost.parentNode) menuHost.parentNode.removeChild(menuHost);
    } catch (e) {
      /* the page may have rewritten the DOM under us */
    }
    menuHost = null;
  }

  function copyToClipboard(text) {
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text);
        return;
      }
    } catch (e) {
      /* fall through to the legacy path */
    }
    try {
      var ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('style', 'position:fixed;top:-1000px;opacity:0');
      document.documentElement.appendChild(ta);
      ta.select();
      document.execCommand('copy');
      document.documentElement.removeChild(ta);
    } catch (e2) {
      log('copy failed', String(e2));
    }
  }

  // Our own link menu, because WKWebView has no "Open Link in New Tab" item
  // and the page has no Tauri bridge with which to ask the React shell for
  // one. Rendered in a shadow root so the page's CSS cannot restyle or hide
  // it, and only for http(s) anchors so ordinary right-clicks (text selection,
  // images, a page's own menu) are left completely alone.
  function showMenu(url, x, y) {
    closeMenu();
    var host = document.createElement('div');
    host.setAttribute('data-permagent-link-menu', '');
    host.setAttribute(
      'style',
      'position:fixed;left:0;top:0;width:0;height:0;z-index:2147483647;',
    );
    var root = host.attachShadow ? host.attachShadow({ mode: 'closed' }) : host;

    var menu = document.createElement('div');
    menu.setAttribute(
      'style',
      'position:fixed;min-width:210px;padding:4px;border-radius:8px;' +
        'background:#1c1c1e;color:#f2f2f7;border:1px solid rgba(255,255,255,0.14);' +
        'box-shadow:0 8px 28px rgba(0,0,0,0.45);font:13px -apple-system,BlinkMacSystemFont,' +
        '"Segoe UI",sans-serif;user-select:none;',
    );

    function addItem(label, onPick) {
      var item = document.createElement('div');
      item.textContent = label;
      item.setAttribute(
        'style',
        'padding:6px 10px;border-radius:5px;cursor:default;white-space:nowrap;',
      );
      item.addEventListener('mouseenter', function () {
        item.style.background = 'rgba(255,255,255,0.12)';
      });
      item.addEventListener('mouseleave', function () {
        item.style.background = 'transparent';
      });
      item.addEventListener('mouseup', function (ev) {
        ev.preventDefault();
        ev.stopPropagation();
        closeMenu();
        onPick();
      });
      menu.appendChild(item);
      return item;
    }

    addItem('Open Link in New Tab', function () {
      openInNewTab(url, 'context-menu');
    });
    addItem('Open Link', function () {
      try {
        location.href = url;
      } catch (e) {
        log('same-tab open failed', String(e));
      }
    });
    addItem('Copy Link', function () {
      copyToClipboard(url);
    });

    root.appendChild(menu);
    (document.body || document.documentElement).appendChild(host);
    menuHost = host;

    // Clamp into the viewport so a link near the right or bottom edge still
    // shows a usable menu.
    try {
      var w = menu.offsetWidth || 210;
      var h = menu.offsetHeight || 96;
      var maxX = (window.innerWidth || 0) - w - 4;
      var maxY = (window.innerHeight || 0) - h - 4;
      menu.style.left = Math.max(4, Math.min(x, maxX)) + 'px';
      menu.style.top = Math.max(4, Math.min(y, maxY)) + 'px';
    } catch (e) {
      menu.style.left = x + 'px';
      menu.style.top = y + 'px';
    }
  }

  // Names of the frames THIS document owns. Read fresh on every gesture rather
  // than cached: single-page apps mount and unmount frames as you navigate, and
  // a stale list would send a click to a tab or a frame that no longer matches
  // the page. Same-document only — reading into a cross-origin child would
  // throw, and its names are not ours to target anyway.
  function frameNamesInDocument() {
    var out = [];
    try {
      var nodes = document.querySelectorAll('iframe[name], frame[name], object[name]');
      for (var i = 0; i < nodes.length; i++) {
        var n = nodes[i].getAttribute ? nodes[i].getAttribute('name') : '';
        if (n) out.push(n);
      }
    } catch (e) {
      /* a page may have replaced querySelectorAll */
    }
    return out;
  }

  // Only a link with a NAMED target can be answered by a frame, and only then
  // is the DOM query worth making. `handle` runs on every mouseup anywhere in
  // somebody else's page; it must stay cheap.
  function needsFrameNames(link) {
    if (!link || !link.target) return false;
    var t = String(link.target).trim().toLowerCase();
    return !!t && t !== '_blank' && t !== '_self' && t !== '_top' && t !== '_parent';
  }

  function context(link) {
    var isTop = true;
    try {
      isTop = window.top === window;
    } catch (e) {
      // Cross-origin parent: we are definitionally not the top frame.
      isTop = false;
    }
    return {
      isTopFrame: isTop,
      frameNames: needsFrameNames(link) ? frameNamesInDocument() : [],
    };
  }

  function handle(ev) {
    var link = __permagentReadLink(__permagentFindAnchor(ev));
    var decision = __permagentLinkDecision(
      {
        type: ev.type,
        button: ev.button,
        metaKey: !!ev.metaKey,
        ctrlKey: !!ev.ctrlKey,
        shiftKey: !!ev.shiftKey,
        defaultPrevented: !!ev.defaultPrevented,
      },
      link,
      context(link),
    );

    if (decision.action === 'ignore') return;

    if (decision.action === 'newtab') {
      // preventDefault, but NOT stopPropagation. A real browser lets the page
      // see the click and only cancels the default action; swallowing the
      // event breaks analytics, tracking pixels and any SPA that legitimately
      // listens on its own links. If a page opens the same URL itself, the
      // shell's duplicate window (see popupTabDecision) still yields one tab.
      ev.preventDefault();
      openInNewTab(decision.url, decision.reason);
      return;
    }

    if (decision.action === 'menu') {
      // The one case that DOES stop propagation: two menus at once helps
      // nobody, and we only reach here for an http(s) anchor — a right-click
      // on text, an image or anything else never gets this far, so the page's
      // own menu and WebKit's (Copy, Look Up, Services) are untouched.
      ev.preventDefault();
      ev.stopPropagation();
      showMenu(decision.url, ev.clientX || 0, ev.clientY || 0);
    }
  }

  // CAPTURE phase, on `window`: we run before the page's own handlers, so a
  // site that swallows clicks cannot take the gesture away — but we only ever
  // CLAIM a gesture the decision function says is a new-tab request, so
  // ordinary left-clicks and SPA routing reach the page untouched.
  window.addEventListener('click', handle, true);
  window.addEventListener('auxclick', handle, true);
  window.addEventListener('contextmenu', handle, true);
  // Middle-click again, from the other end. wry re-dispatches macOS
  // `otherMouseUp` through the left-button path, and not every WebKit build
  // then synthesises an `auxclick`; `mouseup` is the one that always arrives.
  // Whichever lands first wins and the duplicate guard swallows the other —
  // that is precisely what a belt-and-braces path is for.
  window.addEventListener('mouseup', handle, true);
  // Middle-click: cancel the default on the way down too, so no page-level
  // autoscroll or paste behaviour starts before `auxclick` lands.
  window.addEventListener(
    'mousedown',
    function (ev) {
      if (ev.button !== 1) return;
      if (!__permagentFindAnchor(ev)) return;
      ev.preventDefault();
    },
    true,
  );

  // Dismiss the menu the way every menu is dismissed.
  window.addEventListener(
    'mousedown',
    function (ev) {
      if (!menuHost) return;
      if (ev.target === menuHost) return;
      closeMenu();
    },
    true,
  );
  window.addEventListener('scroll', closeMenu, true);
  window.addEventListener('blur', closeMenu, true);
  window.addEventListener(
    'keydown',
    function (ev) {
      if (ev.key === 'Escape') closeMenu();
    },
    true,
  );

  window.__permagentLinks = {
    decide: __permagentLinkDecision,
    openInNewTab: openInNewTab,
    closeMenu: closeMenu,
    hasMenu: function () {
      return !!menuHost;
    },
  };
  log('installed');
  return window.__permagentLinks;
}
