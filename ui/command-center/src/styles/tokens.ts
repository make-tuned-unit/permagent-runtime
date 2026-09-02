/** Permagent design tokens — ported from Claude Design handoff (tokens.js) */

/**
 * THE canonical Permagent neon accent (issue #193, ruled 2026-06-23:
 * global accent = #00D5FF). Every cyan/neon accent in the app derives from
 * this constant — never inline a second near-duplicate (a D9 variant was
 * the historical drift). Non-importable surfaces (index.css fallbacks,
 * iOS Theme.swift) mirror this value literally.
 */
export const NEON_ACCENT = '#00D5FF';

export const color = {
  bg: '#0B1220',
  bgDeeper: '#070B14',
  surface: '#1E2433',
  surfaceHi: '#262D3F',
  border: 'rgba(255,255,255,0.07)',
  borderHi: 'rgba(0,213,255,0.18)',
  cyan: NEON_ACCENT,
  /** The step BELOW cyanSoft: an accent presence you read as atmosphere
   *  rather than as a fill. For the backgrounds of empty, loading and error
   *  states — a panel that is waiting or has nothing to show, tinted enough to
   *  belong to the app and not enough to look like a control. Added
   *  2026-09-02 at the Brain lane's request, on the evidence that six sites
   *  had already written it by hand (Splash, WizardShell, ErrorBoundary,
   *  BrainView x3) at 0.04-0.06, because 0.14 reads as a filled surface. */
  cyanWash: 'rgba(0,213,255,0.04)',
  cyanSoft: 'rgba(0,213,255,0.14)',
  cyanGlow: 'rgba(0,213,255,0.45)',
  purple: '#8D44AE',
  purpleBright: '#A855CC',
  purpleSoft: 'rgba(141,68,174,0.18)',
  purpleGlow: 'rgba(141,68,174,0.45)',
  text: '#FFFFFF',
  textMuted: '#8A94A6',
  textDim: '#5A6478',
  danger: '#FFB4A2',
  /** Strong red for ANSI output and high-emphasis destructive states. */
  dangerStrong: '#EF4444',
} as const;

/**
 * The ink that goes on a bright, saturated fill — a flat cyan button, an amber
 * identity badge, a green trim. Near-black, so it clears 12:1 on any of them;
 * white on a bright fill is the recurring contrast failure this exists to stop
 * (white on #00BFEF is ~1.9:1).
 *
 * One value, one definition, three names' worth of call sites: `textOnCyan`
 * was the first name for it and is kept as an alias in every theme so the
 * existing call sites keep working. New code should say `textOnBright`, which
 * is what it actually means — the ink is a function of the fill's BRIGHTNESS,
 * not of its hue. Added 2026-09-02 for the Finance lane, which had minted a
 * local `FINANCIER_BADGE_INK` because the only token for this was named after
 * a colour its badge is not.
 */
export const INK_ON_BRIGHT = '#04141B';

export const font = {
  display: '"Manrope", "Satoshi", -apple-system, BlinkMacSystemFont, sans-serif',
  body: '"Inter", -apple-system, BlinkMacSystemFont, sans-serif',
  mono: '"JetBrains Mono", ui-monospace, SFMono-Regular, monospace',
} as const;

/**
 * Fixed type ramp (design audit: "no arbitrary sizes"). Role-based — each style
 * bundles size + line-height + weight + tracking, so callers spread one token
 * (`style={{ ...type.body }}`) instead of setting the four ad hoc. Tracking
 * tightens as size grows, per the audit. Sizes in px.
 */
export const type = {
  display: { fontSize: 32, lineHeight: '38px', fontWeight: 600, letterSpacing: '-0.02em' },
  title:   { fontSize: 20, lineHeight: '26px', fontWeight: 600, letterSpacing: '-0.01em' },
  heading: { fontSize: 16, lineHeight: '22px', fontWeight: 600, letterSpacing: '-0.005em' },
  body:    { fontSize: 14, lineHeight: '21px', fontWeight: 400 },
  small:   { fontSize: 13, lineHeight: '19px', fontWeight: 400 },
  caption: { fontSize: 12, lineHeight: '16px', fontWeight: 400 },
  micro:   { fontSize: 11, lineHeight: '14px', fontWeight: 500 },
  label:   { fontSize: 11, lineHeight: '14px', fontWeight: 600, letterSpacing: '0.08em', textTransform: 'uppercase' },
} as const;

/**
 * The ramp's sizes on their own — the same eight roles, nothing added.
 *
 * `type` bundles size with line-height, weight and tracking, and spreading it
 * is the right call when the element is that whole role. But the app also
 * writes a thousand-odd sizes into style objects that already own their weight
 * and their leading (`{ fontSize: 11, color: …, lineHeight: 1.5 }`), and for
 * those, spreading `...type.micro` would change the rendering — it carries
 * `fontWeight: 500` and a 14px leading with it. So they used to re-type the
 * number instead, and the ramp had 1,372 hand-written competitors.
 *
 * `textSize` is the size half, so a caller who needs only the size can still
 * name the role. It is derived from `type` and cannot drift from it. There is
 * no `label` entry: `label` is a role you spread whole, for its tracking and
 * its uppercase; its size is `micro`.
 *
 * Off-ramp sizes (9, 10, 18, 22, 26, 28, 36, and the half-pixels) are NOT here
 * and are not getting an entry. Eight roles is the number; see
 * `textScale.test.ts`, which freezes the ones already written.
 */
export const textSize = {
  display: type.display.fontSize,
  title: type.title.fontSize,
  heading: type.heading.fontSize,
  body: type.body.fontSize,
  small: type.small.fontSize,
  caption: type.caption.fontSize,
  micro: type.micro.fontSize,
} as const;

/** Tabular figures — aligned numerals for metrics/counts/timers (never prose),
 *  so digits don't reflow as values change. Spread onto any numeric display. */
export const tabularNums = { fontVariantNumeric: 'tabular-nums' } as const;

/**
 * Spring easings, sampled as CSS `linear()`.
 *
 * Apple's motion primitive is a spring, not a bezier — SwiftUI ships `.smooth`
 * (bounce 0), `.snappy` (bounce ~0.15) and `.bouncy` (bounce ~0.3), and every
 * bezier is an approximation of one. `linear()` lets us sample the real step
 * response of a damped second-order system, which for UI-scale motion is
 * indistinguishable from the spring itself.
 *
 * These three strings are the unit step response of `zeta = 1 - bounce`,
 * sampled at 25 points and solved so the response is settled (within 0.2% and
 * staying there) exactly at the paired `duration.*` below. That last part is
 * the bit people get wrong: a spring's *perceptual* duration is shorter than
 * its settle time, and putting the perceptual number in CSS clips the tail.
 * Each curve's perceptual duration is noted beside it; the CSS duration is
 * the settle time, and all three are under Apple's 500ms ceiling (HIG:
 * "keep animation duration under 0.5s", "prefer quick, precise animations").
 *
 * They are referenced through `var()` rather than inlined because `linear()`
 * needs Safari 17.2+ and our floor is macOS 11. `index.css` defines the custom
 * properties as beziers and upgrades them to these springs behind
 * `@supports`, so an old WKWebView degrades to a curve we chose instead of to
 * the browser's `ease`. The literal spring strings live here, next to the
 * physics that produced them; `index.css` holds the same values as CSS.
 */
export const SPRING_LINEAR = {
  /** bounce 0, zeta 1.00 — critically damped, no overshoot. The default. */
  smooth: 'linear(0, 0.0493, 0.1576, 0.2855, 0.4117, 0.526, 0.6243, 0.7061, 0.7724, 0.8253, 0.8668, 0.8991, 0.9239, 0.9429, 0.9574, 0.9683, 0.9764, 0.9826, 0.9871, 0.9905, 0.993, 0.9949, 0.9963, 0.9973, 1)',
  /** bounce 0.15, zeta 0.85 — 0.6% overshoot at 180ms. Control state changes. */
  snappy: 'linear(0, 0.0466, 0.1541, 0.2869, 0.4226, 0.5485, 0.658, 0.749, 0.8218, 0.8781, 0.9203, 0.9509, 0.9724, 0.9869, 0.9962, 1.0018, 1.0048, 1.0061, 1.0063, 1.0059, 1.0051, 1.0043, 1.0034, 1.0027, 1)',
  /** bounce 0.30, zeta 0.70 — 4.5% overshoot at 220ms. One or two moments only. */
  bouncy: 'linear(0, 0.0607, 0.2009, 0.3722, 0.5425, 0.6933, 0.8159, 0.9083, 0.9726, 1.0134, 1.0358, 1.045, 1.0453, 1.0405, 1.0331, 1.0251, 1.0176, 1.0113, 1.0063, 1.0027, 1.0003, 0.9988, 0.9981, 0.9979, 1)',
} as const;

export const ease = {
  out: 'cubic-bezier(0.22, 1, 0.36, 1)',
  inOut: 'cubic-bezier(0.65, 0, 0.35, 1)',
  spring: 'cubic-bezier(0.34, 1.56, 0.64, 1)',
  /** Spring, no overshoot. Pair with `duration.smooth` (320ms). */
  smooth: 'var(--pa-ease-smooth, cubic-bezier(0.22, 1, 0.36, 1))',
  /** Spring, slight overshoot. Pair with `duration.snappy` (240ms). */
  snappy: 'var(--pa-ease-snappy, cubic-bezier(0.34, 1.4, 0.64, 1))',
  /** Spring, visible overshoot. Pair with `duration.bouncy` (440ms). */
  bouncy: 'var(--pa-ease-bouncy, cubic-bezier(0.34, 1.56, 0.64, 1))',
} as const;

/**
 * Motion duration scale (ms). One vocabulary for transitions so surfaces don't
 * hand-roll a grab-bag of 160/200/320 timings (wizard audit #603). `fast` =
 * hover/focus feedback, `base` = element state changes, `slow` = view/crossfade.
 * Pair with an `ease` token: `transition: \`all ${duration.fast}ms ${ease.out}\``.
 *
 * `smooth` / `snappy` / `bouncy` are the settle times of the matching spring in
 * `ease`, and are only correct with that spring — a spring sampled over the
 * wrong duration is a different spring. Use them as a pair:
 * `transition: \`transform ${duration.snappy}ms ${ease.snappy}\``.
 */
export const duration = {
  fast: 160, base: 200, slow: 320,
  smooth: 320, snappy: 240, bouncy: 440,
} as const;

/**
 * Spacing scale, in px — the one the code already writes, written down.
 *
 * There was no scale, and 650 hand-written gaps and paddings in its absence.
 * Measuring them turned up a de-facto ramp the app converged on without ever
 * agreeing to it: 4 / 6 / 8 / 10 / 12 / 16, with 20 and 24 for panel-scale
 * padding. That is a 4pt subdivision of the 8pt grid Apple's own layouts use,
 * which is why it felt right to everyone writing it by hand.
 *
 * Codified here, and frozen by `glassTokens.test.ts`, so screen lanes have
 * something to migrate *to*. The 650 raw values are deliberately NOT migrated
 * in this change — a token nobody consumes yet is still the prerequisite for
 * migrating them, and doing both at once would bury the scale in a diff.
 *
 * `xxs: 2` was added 2026-09-02 on the same evidence and by the same rule, at
 * lane R14's request (browser chrome, chip padding). It is not a preference:
 * counting the tree first, `2` appears in 74 hand-written paddings and gaps
 * (49 single-property, 25 as the vertical half of a `'2px Npx'` shorthand)
 * against 40 for `4` by the same grep. Dense chrome — chips, badges, inline
 * pills — genuinely needs a step below 4, and a scale whose floor is above
 * what the code reaches for is a scale people step outside of. It keeps the
 * 2pt grid the bottom of the scale already runs on (4/6/8/10/12).
 */
export const space = {
  xxs: 2, xs: 4, sm: 6, md: 8, lg: 10, xl: 12, xxl: 16, xxxl: 20, huge: 24,
} as const;

/**
 * Corner radii, rebased to what this codebase actually reaches for.
 *
 * The old scale was 6/10/14/20, and the app's single most-used radius — 8px,
 * by a wide margin — was not in it. Two hundred and sixty-three hand-written
 * values were not written by people ignoring the scale; they were written by
 * people who needed a step it did not have. So the scale moved to meet them:
 * 4/6/8/12/16 is what Linear and Raycast ship, and it is what the developers
 * here converged on by hand.
 *
 * `sm` stays 6, so buttons are untouched. `md`, `lg` and `xl` each tighten by
 * a couple of pixels at their existing call sites — a uniform, deliberate
 * step toward the values the app already preferred, not a per-surface retune.
 *
 * `pill` is for chips. Not for buttons: a pill-shaped CTA is a different
 * product's voice.
 *
 * `glass` is the outermost floating surface, and it is derived rather than
 * chosen. macOS gives our window its corner for free (we keep
 * `decorations: true`), so the topmost glass plane should sit concentrically
 * inside that corner: `radius.glass = concentric(window corner, inset)`.
 *
 * MEASURED 2026-09-01 against the running app — `screencapture -o -l` of the
 * live Permagent window (1202x885 pt, captured at 2x), then a least-squares
 * fit of the alpha edge over the first 40 rows of the top-left corner:
 *
 *   superellipse  R = 18.07pt, n = 2.42   (rms 0.05pt)  ← the real shape
 *   best circle   R = 15.63pt             (rms 0.14pt)  ← what CSS can draw
 *
 * The superellipse fit is 3x tighter, which confirms Apple draws a continuous
 * -curvature corner and not an arc; n = 2.42 sits inside the 2.3-2.6 the A1a
 * spike measured on Tahoe titlebar-only windows, and R = 18.07 inside its
 * 17.4 +/- 2.5. We have no `corner-shape: squircle` in WebKit, so we draw the
 * arc and accept it — sub-perceptual at these radii (research doc, grade 22).
 *
 * That gives two defensible derivations at our 8px inset:
 *   concentric(18.07, 8) = 10   (Apple's arithmetic, on the nominal radius)
 *   concentric(15.63, 8) =  8   (the arithmetic on the arc we actually draw)
 *
 * 9 is between them and within the measurement error of both, so the spike's
 * provisional 9 stands unchanged. Recorded, not adjusted.
 */
export const radius = { xs: 4, sm: 6, md: 8, lg: 12, xl: 16, glass: 9, pill: 999 } as const;

/**
 * Concentric corner radius: `r_inner = max(0, r_outer - padding)`.
 *
 * Apple's rule, verbatim from the SwiftUI reference for
 * `Edge.Corner.Style.concentric`: "the system calculates the corner radius to
 * equal the container shape's corner radius minus the distance between
 * corners", and if the result would be negative "the corner is square".
 *
 * So a 16px-radius panel with 12px padding holds 4px-radius children, and the
 * same panel with 20px padding holds square-cornered ones. The square corner
 * is the correct answer, not a bug to clamp away from — a child rounded more
 * than its container's remaining curvature is the "pinched or flared" failure
 * WWDC25/356 names.
 *
 * Rounded to whole px: a fractional border-radius antialiases into a soft
 * corner, and the whole point of concentricity is that the curves line up.
 */
export function concentric(outer: number, padding: number): number {
  return Math.max(0, Math.round(outer - padding));
}

/**
 * Window-shell geometry — the numbers the CSS shares with AppKit.
 *
 * The macOS window runs `titleBarStyle: "Overlay"` + `hiddenTitle` with
 * `decorations: true`, so the system titlebar is transparent, our HTML runs
 * edge-to-edge underneath it, and macOS still draws the window's corner and
 * shadow for free. That makes the top-left corner of the page a shared space:
 * the traffic lights are native and always composite above everything the
 * webview draws, so the shell has to leave room for them rather than negotiate.
 *
 * These four numbers are that room, and they are load-bearing in both
 * directions — `ui/desktop/src-tauri/src/chrome.rs` holds the same values as
 * Rust constants, `tauri.conf.json` holds them again as the only path that
 * actually applies them (A1a's binding verdict), and `shell.test.ts` fails if
 * the three copies ever disagree.
 *
 * `trafficLights.y` is NOT the distance from the top of the window. AppKit
 * sizes the titlebar container `buttonSize + y` tall and pins it to the top
 * edge, while the button keeps its own 9pt origin inside it — so the visible
 * inset is `y - 9`. y = 22 puts a 14pt button 13pt down, centred in the 40pt
 * `titlebar` band.
 *
 * `rail.collapsed` is 76 rather than the 64 it was before the rail went
 * full-height: the three window buttons span `x + 60 = 72pt` and now sit
 * INSIDE the rail, so anything narrower hangs the zoom button over the rail's
 * edge onto the content pane. That constraint is what fixed x at 12.
 */
export const shell = {
  /** Height of the titlebar band, in the sidebar and over the content alike. */
  titlebar: 40,
  /** Native window-control geometry. Mirrors `chrome.rs` / `tauri.conf.json`. */
  trafficLights: { x: 12, y: 22, buttonSize: 14, spacing: 23 },
  /** Sidebar rail width, open and collapsed. */
  rail: { open: 208, collapsed: 76 },
} as const;

/** Window-left edge to the right edge of the zoom button, in px. */
export function trafficLightSpan(x: number = shell.trafficLights.x): number {
  const { spacing, buttonSize } = shell.trafficLights;
  return x + 2 * spacing + buttonSize;
}

export const shadow = {
  glow: '0 0 40px rgba(0,213,255,0.25)',
  glowStrong: '0 0 80px rgba(0,213,255,0.4)',
  card: '0 8px 32px rgba(0,0,0,0.5), 0 0 0 1px rgba(255,255,255,0.04)',
} as const;

export const tokens = { color, font, type, textSize, tabularNums, ease, duration, space, radius, shadow, shell } as const;
export type DesignTokens = typeof tokens;

// ── Theme gradients + colors ────────────────────────────────────────
export type ThemeId = 'dark' | 'aurora' | 'silver';

/** Per-theme color overrides. Components use useTheme().colors for theme-aware colors. */
export interface ThemeColors {
  bg: string; bgDeeper: string; surface: string; surfaceHi: string;
  border: string; borderHi: string;
  cyan: string; cyanWash: string; cyanSoft: string; cyanGlow: string;
  purple: string; purpleBright: string; purpleSoft: string; purpleGlow: string;
  text: string; textMuted: string; textDim: string;
  danger: string; dangerStrong: string;
  /** Card elevation shadow (cool-tinted on silver) */
  cardShadow: string;
  /** Discrete elevation ladder for floating layers (theme-aware — deep on the
   *  void, soft-cool on silver where a black shadow is invisible). Use the level
   *  that matches prominence: raised=dropdowns/menus, overlay=popovers/panels,
   *  floating=toasts/notifications. Static content stays border-first, flat. */
  elevationRaised: string;
  elevationOverlay: string;
  elevationFloating: string;
  /** Top-edge highlight for metallic cards (empty string on dark themes) */
  cardHighlight: string;
  /** Brand ribbon gradient for primary buttons / AI moments */
  ribbonGradient: string;
  /** User chat bubble surface */
  userBubble: string;
  /** User chat bubble text */
  userBubbleText: string;
  /** Inset surface for inputs */
  inputBg: string;
  /** On-accent text (white on the blue→violet gradient/ribbon buttons) */
  textOnAccent: string;
  /** Text/iconography sitting on a FLAT cyan accent fill (solid `colors.cyan`
   *  buttons, active pills). Cyan is bright in every theme, so on-cyan text must
   *  be a fixed dark ink — white/`colors.bg` fails WCAG contrast (and inverts to
   *  near-white on the silver theme). Never use textOnAccent on a flat-cyan fill. */
  textOnCyan: string;
  /** The same ink as `textOnCyan`, under the name that says what it is for:
   *  any bright, saturated fill, whatever its hue. Prefer this. */
  textOnBright: string;
  /** Success semantic */
  success: string;
  /** Warning semantic */
  warning: string;
  /** Staleness semantic — a figure whose AGE is the thing worth noticing.
   *  Deliberately not `warning`: nothing is wrong with a three-month-old
   *  memory or a dashboard that lost its poll, they are just no longer current,
   *  and a console that shouts about age has nothing left for a real fault.
   *  Aged amber: clearly out of the quiet range, clearly not an alarm. */
  stale: string;
  /** Tint of `stale` for the surface behind it (a chip fill, a caption's dot). */
  staleSoft: string;
  /** Inline code background */
  codeBg: string;
  /** Inline code text */
  codeText: string;
  /**
   * Neutral interaction fills — the theme-safe replacement for the
   * `rgba(255,255,255,0.06)` idiom, which is written 202 times across the app
   * and is *invisible* on silver: white at 6% over a white surface is white.
   * These carry the theme's own ink, so the same token reads as a lift on the
   * void and as a shade on the pearl.
   *
   * `fillSubtle` = resting tint (a zebra row, an inactive chip).
   * `fillHover` / `fillActive` = the pointer ladder every interactive element
   * owes a Mac user (D10). Use them for the *fill*; brightness/scale carry the
   * rest of the press feedback.
   */
  fillSubtle: string;
  fillHover: string;
  fillActive: string;
  /**
   * Scrim behind a modal or a dismissable overlay — the layer that takes the
   * content behind it out of contention. Not a glass surface and never
   * blurred: it is the thing glass would sit *on*.
   */
  veil: string;
}

/**
 * One glass material, as the four properties a surface needs to wear it.
 *
 * `background` is translucent and `backdropFilter` is real only because these
 * two always travel together — the app's standing bug was a `backdropFilter`
 * paired with an OPAQUE `colors.surface`, which blurs nothing at a full
 * compositing pass's cost. Spread the whole token or none of it.
 *
 * `opaque` is the same surface with the translucency taken out, for Reduce
 * Transparency. It is the theme's flat surface colour, deliberately: under
 * that setting `glass` and `glassHi` collapse to the same fill, which is what
 * the setting means.
 */
export interface GlassSurface {
  background: string;
  backdropFilter: string;
  /** Specular top edge, shaded bottom edge, hairline, ambient depth. */
  boxShadow: string;
  /** Reduce-Transparency fallback fill (see `getReduceTransparency`). */
  opaque: string;
}

/**
 * The glass token set — two surfaces, and that is the whole vocabulary.
 *
 * `glass` is the default floating-control material: toolbars, popovers,
 * toasts, floating buttons, menu chrome. `glassHi` is for LARGE floating
 * surfaces — sidebars, inspectors — and is MORE opaque, which is the rule
 * people find backwards. It is Apple's: large glass "uses increased opacity to
 * preserve legibility over complex backgrounds" and refuses the polarity flip
 * because at that size the flip is distracting (WWDC25/219). Bigger surface,
 * more opaque.
 *
 * Both sit at Tinted-appearance opacity, not at the June-2025 launch
 * transparency. Apple spent 26.1, 26.2 and 26.4 walking that transparency back
 * and shipped a user-facing switch to turn it down; starting where they
 * started would be starting a year behind.
 *
 * There is no `clear` variant here. Apple's Clear is for controls over
 * media-rich content, and the one place we could justify it — HUD chrome over
 * the 3D world view — is a screen this change deliberately does not touch. A
 * third glass token with no caller is a third thing to keep honest; the world
 * lane can add it against a real backdrop.
 */
export interface ThemeGlass { glass: GlassSurface; glassHi: GlassSurface; }

const DARK_COLORS: ThemeColors = {
  bg: color.bg, bgDeeper: color.bgDeeper, surface: color.surface, surfaceHi: color.surfaceHi,
  border: color.border, borderHi: color.borderHi,
  cyan: color.cyan, cyanWash: color.cyanWash, cyanSoft: color.cyanSoft, cyanGlow: color.cyanGlow,
  purple: color.purple, purpleBright: color.purpleBright, purpleSoft: color.purpleSoft, purpleGlow: color.purpleGlow,
  text: color.text, textMuted: color.textMuted, textDim: color.textDim,
  danger: color.danger,
  dangerStrong: color.dangerStrong,
  cardShadow: '0 8px 32px rgba(0,0,0,0.5), 0 0 0 1px rgba(255,255,255,0.04)',
  elevationRaised: '0 4px 16px rgba(0,0,0,0.35)',
  elevationOverlay: '0 8px 24px rgba(0,0,0,0.42)',
  elevationFloating: '0 20px 52px rgba(0,0,0,0.5)',
  cardHighlight: '',
  ribbonGradient: 'linear-gradient(135deg, #00D5FF 0%, #6366F1 50%, #8D44AE 100%)',
  userBubble: 'rgba(141,68,174,0.18)',
  userBubbleText: '#FFFFFF',
  inputBg: '#1E2433',
  textOnAccent: '#FFFFFF',
  textOnCyan: INK_ON_BRIGHT,
  textOnBright: INK_ON_BRIGHT,
  success: '#34D399',
  warning: '#FBBF24',
  // 8.6:1 on the dark ground, and a hue nobody mistakes for the amber alarm.
  stale: '#D0A45C',
  staleSoft: 'rgba(208,164,92,0.16)',
  codeBg: 'rgba(0,0,0,0.30)',
  codeText: '#00D5FF',
  // White ink on the void. The ladder is 4/7/11 — 7 is `color.border`'s alpha
  // on purpose, so a hovered fill and a hairline are the same weight of light.
  fillSubtle: 'rgba(255,255,255,0.04)',
  fillHover: 'rgba(255,255,255,0.07)',
  fillActive: 'rgba(255,255,255,0.11)',
  // bgDeeper (#070B14) at 62% — deep enough that a card behind a modal stops
  // competing, short of the flat blackout that loses all sense of place.
  veil: 'rgba(7,11,20,0.62)',
};
const AURORA_COLORS: ThemeColors = {
  ...DARK_COLORS,
  ribbonGradient: 'linear-gradient(135deg, #00D5FF 0%, #6366F1 50%, #A855CC 100%)',
};
const SILVER_COLORS: ThemeColors = {
  // Surfaces — Pearl White base, white glass cards
  bg: '#F8FAFC',             // Pearl White (page background)
  bgDeeper: '#EEF2F7',      // Chrome Mist (secondary/inset bg)
  surface: '#FFFFFF',        // Pure white glass cards
  surfaceHi: '#F8FAFC',     // Elevated (modals, popovers)
  // Borders — Titanium Gray hairline
  border: 'rgba(167,176,190,0.35)',
  borderHi: 'rgba(0,191,239,0.40)',  // Cyan focus
  // Accents — brand cyan/violet
  cyan: '#00BFEF',           // Cyan Intelligence
  // Same step below Soft as the dark theme takes (0.04 x 0.10/0.14), because
  // silver dials the whole cyan family down by that ratio already.
  cyanWash: 'rgba(0,191,239,0.03)',
  cyanSoft: 'rgba(0,191,239,0.10)',
  cyanGlow: 'rgba(0,191,239,0.25)',
  purple: '#8B5CFF',         // Violet Memory
  purpleBright: '#9B6FFF',
  purpleSoft: 'rgba(139,92,255,0.10)',
  purpleGlow: 'rgba(139,92,255,0.25)',
  // Text — Graphite
  text: '#1E2530',           // Graphite Text (primary) — 14.5:1 on white
  textMuted: '#4B5563',      // Cool grey — 7.0:1 on white (AA)
  textDim: '#6B7585',        // Titanium Gray — 4.9:1 on white (AA body text)
  // Semantic
  danger: '#DC2626',
  dangerStrong: '#DC2626',
  // Elevation — soft shadow + glass edge (cards MUST float via shadow, not color)
  cardShadow: '0 2px 12px rgba(30,37,48,0.10), 0 1px 4px rgba(30,37,48,0.06)',
  elevationRaised: '0 4px 16px rgba(30,37,48,0.10)',
  elevationOverlay: '0 8px 24px rgba(30,37,48,0.13)',
  elevationFloating: '0 20px 52px rgba(30,37,48,0.17)',
  cardHighlight: 'inset 0 1px 0 rgba(255,255,255,0.9)',
  // Brand ribbon — weighted so text sits over blue→violet (AA large text)
  ribbonGradient: 'linear-gradient(135deg, #00BFEF 0%, #3A7BFF 30%, #8B5CFF 100%)',
  userBubble: '#DCCBFF',     // Soft Lavender
  userBubbleText: '#1E2530', // Graphite (BLACK text)
  inputBg: '#EEF2F7',       // Chrome Mist (recessed)
  textOnAccent: '#FFFFFF',
  textOnCyan: INK_ON_BRIGHT,  // ~12:1 on #00BFEF (white would be ~1.9:1)
  textOnBright: INK_ON_BRIGHT,
  success: '#059669',
  warning: '#D97706',
  // Amber-700 — 4.9:1 on white (AA), a step deeper than the warning amber so
  // age still reads as age on the light theme.
  stale: '#A16207',
  staleSoft: 'rgba(161,98,7,0.10)',
  codeBg: '#EEF2F7',          // Chrome Mist — 1.1:1 vs white (subtle tint)
  codeText: '#0369A1',        // Sky-700 — 5.5:1 on Chrome Mist (AA)
  // Graphite ink (30,37,48 = `text`) at the same 4/7/11 ladder. This is the
  // half that was missing: white-alpha fills are literally invisible here, so
  // every hover written that way simply did not exist on the light theme.
  fillSubtle: 'rgba(30,37,48,0.04)',
  fillHover: 'rgba(30,37,48,0.07)',
  fillActive: 'rgba(30,37,48,0.11)',
  // Lighter than the dark theme's veil, and correctly so — on a pearl ground a
  // 62% graphite scrim reads as a blackout, not as a recede.
  veil: 'rgba(30,37,48,0.40)',
};

// ── Glass ────────────────────────────────────────────────────────────

/** `#RRGGBB` -> `rgba(r,g,b,a)`. Solid hex only; the theme surfaces all are. */
function _rgba(hex: string, alpha: number): string {
  const s = hex.replace('#', '');
  const r = parseInt(s.substring(0, 2), 16);
  const g = parseInt(s.substring(2, 4), 16);
  const b = parseInt(s.substring(4, 6), 16);
  return `rgba(${r},${g},${b},${alpha})`;
}

/**
 * Build a theme's two glass surfaces from its own opaque surface colour.
 *
 * Derived, not hand-written, for one reason: glass must read as the same
 * material family as the flat surfaces beside it. If a theme ever retunes
 * `surface`, its glass follows in the same commit instead of drifting a shade
 * off and needing to be noticed. (Aurora currently shares dark's surface, so
 * it currently shares dark's glass — and will stop the moment it stops.)
 *
 * `light` inverts the rim polarity rather than the whole recipe: the specular
 * highlight stays on TOP in both — light comes from above in both themes —
 * but on a pearl ground the bottom edge softens instead of darkening, and the
 * ambient shadow is cool graphite, because black is invisible on white.
 */
function _glass(surface: string, border: string, light: boolean): ThemeGlass {
  const rim = light
    ? ['inset 0 1px 0 rgba(255,255,255,0.90)', 'inset 0 -1px 0 rgba(30,37,48,0.06)']
    : ['inset 0 1px 0 rgba(255,255,255,0.14)', 'inset 0 -1px 0 rgba(0,0,0,0.22)'];
  const ambient = (spread: string) => (light ? `0 ${spread} rgba(30,37,48,0.12)` : `0 ${spread} rgba(0,0,0,0.28)`);
  const shell = (spread: string) => [...rim, `inset 0 0 0 1px ${border}`, ambient(spread)].join(', ');
  return {
    // Default floating control layer — toolbars, popovers, toasts, launchers.
    glass: {
      background: _rgba(surface, 0.82),
      backdropFilter: 'blur(20px) saturate(180%)',
      boxShadow: shell('8px 32px'),
      opaque: surface,
    },
    // Large floating surfaces — sidebars, inspectors. More opaque (D6), and a
    // wider blur and deeper ambient because it floats further off the content.
    glassHi: {
      background: _rgba(surface, 0.90),
      backdropFilter: 'blur(24px) saturate(170%)',
      boxShadow: shell('16px 48px'),
      opaque: surface,
    },
  };
}

/**
 * `saturate()` is doing real work in both filters and is not decoration:
 * `blur()` alone averages the backdrop toward grey haze, and it is the
 * saturation boost that makes the result read as glass rather than as fog.
 * Dropping it is the single most common way a web recreation looks wrong.
 */
export const THEME_GLASS: Record<ThemeId, ThemeGlass> = {
  dark: _glass(DARK_COLORS.surface, DARK_COLORS.border, false),
  aurora: _glass(AURORA_COLORS.surface, AURORA_COLORS.border, false),
  silver: _glass(SILVER_COLORS.surface, SILVER_COLORS.border, true),
};

export function getThemedGlass(): ThemeGlass { return THEME_GLASS[_activeTheme]; }

export interface ThemeGradients {
  workspace: string; card: string; label: string;
  shell: string; sidebar: string; navRail: string;
  dropdown: string; dropdownSolid: string;
}

export const THEME_GRADIENTS: Record<ThemeId, ThemeGradients> = {
  dark: {
    workspace: 'radial-gradient(120% 80% at 50% 0%, #142035 0%, #0B1220 50%, #050810 100%)',
    card: 'linear-gradient(180deg, rgba(20,28,48,0.7), rgba(11,18,32,0.7))',
    shell: '#0B1220',
    sidebar: 'rgba(7,11,20,0.6)',
    navRail: 'rgba(7,11,20,0.4)',
    dropdown: 'rgba(11,18,32,0.98)',
    dropdownSolid: '#0B1220',
    label: 'Permagent dark',
  },
  aurora: {
    workspace: 'radial-gradient(120% 80% at 50% 0%, #1a1040 0%, #0B1220 40%, #2d1050 100%)',
    card: 'linear-gradient(180deg, rgba(45,16,80,0.5), rgba(11,18,32,0.7))',
    shell: '#0e0a1e',
    sidebar: 'rgba(14,10,30,0.7)',
    navRail: 'rgba(14,10,30,0.5)',
    dropdown: 'rgba(14,10,30,0.98)',
    dropdownSolid: '#0e0a1e',
    label: 'Aurora',
  },
  silver: {
    workspace: 'linear-gradient(180deg, #F8FAFC 0%, #F2F5F9 100%)',
    card: 'linear-gradient(180deg, #FFFFFF 0%, #FAFBFD 100%)',
    shell: '#FFFFFF',         // Pure white header (matches content, separated by border)
    sidebar: 'rgba(248,250,252,0.95)',
    navRail: 'rgba(248,250,252,0.85)',
    dropdown: 'rgba(255,255,255,0.98)',
    dropdownSolid: '#FFFFFF',
    label: 'Silver',
  },
};

const THEME_COLORS: Record<ThemeId, ThemeColors> = {
  dark: DARK_COLORS,
  aurora: AURORA_COLORS,
  silver: SILVER_COLORS,
};

export function getThemedColors(): ThemeColors { return THEME_COLORS[_activeTheme]; }

// ── Reactive appearance prefs ────────────────────────────────────────
// Persisted to localStorage, reactive via listener set.

const _listeners: Set<() => void> = new Set();
function _notify() { _listeners.forEach(fn => fn()); }
function _get(key: string, fallback: string): string {
  try { return localStorage.getItem(key) ?? fallback; } catch { return fallback; }
}
function _set(key: string, value: string) {
  try { localStorage.setItem(key, value); } catch { /* */ }
  _notify();
}

// Theme. The stored PREFERENCE may be 'system' (light-awareness, à la
// ChatGPT/Claude): follow the OS — silver during light hours, dark when the
// device is dark, live-switching when the OS does. The resolved _activeTheme
// is always a concrete ThemeId so every consumer keeps working.
export type ThemePref = ThemeId | 'system';

const THEME_PREFS: readonly ThemePref[] = ['dark', 'aurora', 'silver', 'system'];

/** Keep old or manually-edited preferences from producing an undefined token set. */
export function normalizeThemePref(value: string | null): ThemePref {
  if (value === 'slate') return 'silver';
  return THEME_PREFS.includes(value as ThemePref) ? value as ThemePref : 'dark';
}

function _prefersDark(): boolean {
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-color-scheme: dark)').matches;
}

function _resolve(pref: ThemePref): ThemeId {
  if (pref === 'system') return _prefersDark() ? 'dark' : 'silver';
  return pref;
}

const _storedThemePref = _get('permagent-theme', 'dark');
let _themePref: ThemePref = normalizeThemePref(_storedThemePref);
// Migrate 'slate' -> 'silver' (one-time, idempotent)
if (_storedThemePref === 'slate') {
  _set('permagent-theme', 'silver');
}
let _activeTheme: ThemeId = _resolve(_themePref);
export function getTheme(): ThemeId { return _activeTheme; }
export function getThemePref(): ThemePref { return _themePref; }
export function getThemeGradient() { return THEME_GRADIENTS[_activeTheme]; }
export function setTheme(pref: ThemePref) {
  _themePref = pref;
  _activeTheme = _resolve(pref);
  _set('permagent-theme', pref);
}

// Live OS-theme switching: when the preference is 'system', re-resolve and
// notify on every prefers-color-scheme flip (the day/night shift).
if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    if (_themePref === 'system') {
      _activeTheme = _resolve('system');
      _notify();
    }
  });
}

// Extract RGB channel triplet from a hex color string for Tailwind alpha-modifier support.
// e.g. '#0B1220' → '11 18 32'. Only call on solid hex values, not rgba().
function _hex(h: string): string {
  const s = h.replace('#', '');
  return `${parseInt(s.substring(0, 2), 16)} ${parseInt(s.substring(2, 4), 16)} ${parseInt(s.substring(4, 6), 16)}`;
}

// Sync CSS custom properties for Tailwind theme-aware colors.
// Solid colors are stored as RGB channel triplets so Tailwind /NN alpha modifiers
// compose correctly: rgb(var(--tw-dark-muted) / 0.5). Colors with intrinsic alpha
// (border, glow) are stored as full rgba values and don't support further alpha.
function _syncCssVars() {
  if (typeof document === 'undefined') return;
  const c = THEME_COLORS[_activeTheme];
  const root = document.documentElement.style;
  // Channel triplets (support Tailwind /NN alpha modifiers)
  root.setProperty('--tw-dark-bg', _hex(c.bg));
  root.setProperty('--tw-dark-surface', _hex(c.surface));
  root.setProperty('--tw-dark-surface-2', _hex(c.surfaceHi));
  root.setProperty('--tw-dark-text', _hex(c.text));
  root.setProperty('--tw-dark-muted', _hex(c.textMuted));
  root.setProperty('--tw-accent', _hex(c.cyan));
  root.setProperty('--tw-accent-dim', _hex(c.cyan));
  root.setProperty('--tw-input-bg', _hex(c.inputBg));
  root.setProperty('--tw-danger', _hex(c.danger));
  root.setProperty('--tw-success', _hex(c.success));
  root.setProperty('--tw-warning', _hex(c.warning));
  // Status palette → bridged to theme tokens (ok→success, warn→warning, error→danger, info→cyan)
  root.setProperty('--tw-status-ok', _hex(c.success));
  root.setProperty('--tw-status-warn', _hex(c.warning));
  root.setProperty('--tw-status-error', _hex(c.danger));
  root.setProperty('--tw-status-info', _hex(c.cyan));
  // Full rgba values (intrinsic alpha, no Tailwind alpha modifier support)
  root.setProperty('--tw-dark-border', c.border);
  root.setProperty('--tw-accent-glow', c.cyanSoft);
  // Scrollbar colors per theme (aligned with tokens.ts dark bg/surface)
  const scrollThumb = _activeTheme === 'silver' ? '#C8CDD5' : '#1E2433';
  const scrollThumbHover = _activeTheme === 'silver' ? '#A0A8B4' : '#262D3F';
  root.setProperty('--scrollbar-thumb', scrollThumb);
  root.setProperty('--scrollbar-thumb-hover', scrollThumbHover);
  // Sync color-scheme + body background so macOS native title bar matches theme
  const scheme = _activeTheme === 'silver' ? 'light' : 'dark';
  root.setProperty('color-scheme', scheme);
}
_syncCssVars(); // initial sync
_listeners.add(_syncCssVars); // re-sync on theme change

// Cross-window theme sync: listen for localStorage changes from other windows
// (e.g., chat window picks up theme change made in main window's Settings)
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === 'permagent-theme') {
      _themePref = normalizeThemePref(e.newValue);
      _activeTheme = _resolve(_themePref);
      _notify();
    }
  });
}

// Möbius glow (0-100)
export function getMobiusGlow(): number { return parseInt(_get('permagent-mobius-glow', '70'), 10); }
export function setMobiusGlow(v: number) { _set('permagent-mobius-glow', String(v)); }

// Möbius idle animation: 'still' | 'breathing' | 'drifting'
export type IdleAnim = 'still' | 'breathing' | 'drifting';
export function getIdleAnim(): IdleAnim { return _get('permagent-idle-anim', 'breathing') as IdleAnim; }
export function setIdleAnim(v: IdleAnim) { _set('permagent-idle-anim', v); }

// UI density: 'comfortable' | 'default' | 'compact'
export type UIDensity = 'comfortable' | 'default' | 'compact';
export function getDensity(): UIDensity { return _get('permagent-density', 'default') as UIDensity; }
export function setDensity(v: UIDensity) { _set('permagent-density', v); }

// Reduce motion
// Three states, not two. An explicit choice in settings always wins — that is
// the whole point of having the setting. But when nobody has chosen, the
// honest default is the one the person already gave their operating system:
// someone who turned on Reduce Motion in macOS has answered this question
// once and should not have to answer it again per app. The World reads this in
// eighteen places, so this is also what makes the 3D scene calm for them.
export function getReduceMotion(): boolean {
  const stored = _get('permagent-reduce-motion', '');
  if (stored === 'true') return true;
  if (stored === 'false') return false;
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}
export function setReduceMotion(v: boolean) { _set('permagent-reduce-motion', String(v)); }

// Reduce transparency
//
// The same three states as Reduce Motion — explicit choice wins, otherwise
// follow the OS — but the OS half cannot be read the same way. WebKit does not
// implement `prefers-reduced-transparency` (it was objected to on
// fingerprinting grounds), so inside our webview the setting is invisible to
// CSS and to matchMedia alike. There is no media query to fall through to.
//
// So the OS answer is pushed IN: `styles/reduceTransparency.ts` reads
// `NSWorkspace.accessibilityDisplayShouldReduceTransparency` over Tauri IPC and
// calls `setNativeReduceTransparency` here, once at startup and again on every
// accessibility-options change. Until that lands (a browser, a test, the split
// second before the first IPC returns) the answer is `false` — glass on. That
// is the right default to be wrong in: a user who has the setting ON gets one
// frame of glass, not a permanently flat app for everyone else.
let _nativeReduceTransparency = false;

/** Called by the native bridge. Notifies theme listeners so glass re-renders. */
export function setNativeReduceTransparency(v: boolean) {
  if (_nativeReduceTransparency === v) return;
  _nativeReduceTransparency = v;
  _notify();
}

export function getReduceTransparency(): boolean {
  const stored = _get('permagent-reduce-transparency', '');
  if (stored === 'true') return true;
  if (stored === 'false') return false;
  return _nativeReduceTransparency;
}
export function setReduceTransparency(v: boolean) { _set('permagent-reduce-transparency', String(v)); }

export function onThemeChange(fn: () => void) { _listeners.add(fn); return () => { _listeners.delete(fn); }; }
