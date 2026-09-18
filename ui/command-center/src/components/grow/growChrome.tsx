/**
 * Grow's non-button interaction chrome.
 *
 * `.pa-btn` already gives every BUTTON on this screen the pointer ladder a Mac
 * user is owed — hover fill, hover border, hover ink, a 0.97 press. Grow's
 * other twenty-five interactive elements got none of it: ten `<select>`s, a
 * dozen `<input>`/`<textarea>`s, three `<details>` summaries and the `site ↗` /
 * `repo ↗` links. They had the app-wide `:focus-visible` ring and nothing else,
 * so on a pointer platform they were inert until clicked. D10: "Every
 * interactive element has a visible hover state… This is the cheapest single
 * change that makes a web UI feel like a Mac app."
 *
 * Done as three CSS rules rather than twenty pairs of `onMouseEnter`/
 * `onMouseLeave`, for the reason `.pa-btn` is CSS: a hover written in React
 * state is a re-render per pointer move, and it cannot express `:disabled` or
 * `:focus-within` at all.
 *
 * The colours arrive as custom properties from the theme, exactly as `.pa-btn`
 * takes them, so this file hard-codes no palette — which is what lets one rule
 * set be correct on the void and on the pearl. `fillHover` carries the theme's
 * own ink, so the same token reads as a lift on dark and a shade on silver —
 * where the white-alpha idiom it replaces app-wide (white at 6% over a white
 * surface) is literally invisible.
 *
 * Scoped to `[data-grow-chrome]` so it cannot reach another screen. Mounted
 * once by `GrowView`; the constants below are what the elements opt in with.
 */

import type { CSSProperties } from 'react';
import { concentric, duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';

/** Put on any `<select>`, `<input>` or `<textarea>` in this directory. */
export const FIELD_CLASS = 'pa-grow-field';
/** Put on a `<summary>` — the disclosure rows on Archived / Dismissed / provenance. */
export const SUMMARY_CLASS = 'pa-grow-summary';
/** Put on an `<a>` that is a real navigation, not a control. */
export const LINK_CLASS = 'pa-grow-link';

/**
 * The one text-field look on this screen.
 *
 * There were ten copies of it — `field` in PostizConnect, `field` in
 * HiggsfieldConnect, `field` in BrandCard, `select` in ActionVerify,
 * `fieldStyle` in AnalyticsConnectForm, and five written inline in the calendar
 * row — agreeing on roughly everything and differing in radius, padding and
 * font by a pixel each. One concept, one place.
 *
 * `inputBg` rather than `bgDeeper`: the theme has a token for "a recessed
 * surface a value is typed into", and using the page's deepest background for
 * it made a field and an empty region look identical.
 */
export function growField(colors: ThemeColors, opts?: { mono?: boolean }): CSSProperties {
  return {
    background: colors.inputBg,
    color: colors.text,
    border: `1px solid ${colors.border}`,
    borderRadius: radius.sm,
    padding: `${space.sm}px ${space.md}px`,
    fontSize: textSize.caption,
    fontFamily: opts?.mono ? font.mono : font.body,
    boxSizing: 'border-box',
    // Hover/focus colours live in the stylesheet; these are the transitions
    // they ride in on. Paired duration and curve — a spring sampled over the
    // wrong duration is a different spring.
    transition: `background-color ${duration.fast}ms ${ease.smooth}, `
      + `border-color ${duration.fast}ms ${ease.smooth}`,
  };
}

/**
 * The uppercase mono micro-label this screen uses about forty times: a section
 * heading, a window name, a provenance chip's text.
 *
 * `type.label` IS this role — 11px, 600, 0.08em, uppercase — and every one of
 * these sites had re-typed its four properties by hand at 9px or 10px. Spread
 * the role and add only the two things the role does not carry.
 */
export function growLabel(colors: ThemeColors, color?: string): CSSProperties {
  return {
    fontFamily: font.mono,
    fontSize: textSize.micro,
    letterSpacing: '0.08em',
    textTransform: 'uppercase',
    color: color ?? colors.textDim,
  };
}

/**
 * A content card: opaque fill, one hairline, no shadow.
 *
 * Content, so never glass — Apple's single most-stated rule, and the one that
 * decides whether this reads as native or as a theme. Elevation comes from the
 * fill and the hairline, which is where D1 says it should come from, and the
 * `shadow.card` a web app would reach for here is a web convention rather than
 * an Apple one (apple.com's whole design language uses exactly one drop shadow,
 * on product photography).
 *
 * `r` and `pad` travel together so a caller can derive a child's radius with
 * `concentric(r, pad)` from the same two numbers, rather than picking a second
 * one by eye.
 */
export function growCard(
  colors: ThemeColors,
  opts?: { r?: number; pad?: number; accent?: boolean },
): CSSProperties {
  const r = opts?.r ?? radius.lg;
  const pad = opts?.pad ?? space.xxl;
  return {
    background: colors.surface,
    border: `1px solid ${opts?.accent ? colors.borderHi : colors.border}`,
    borderRadius: r,
    padding: pad,
  };
}

/** The radius a child of `growCard(colors, { r, pad })` gets. See D4. */
export function growCardInner(r = radius.lg, pad = space.xxl): number {
  return concentric(r, pad);
}

/**
 * The stylesheet. Mounted once, by the view.
 *
 * `:disabled` is excluded from every hover, because a control that lights up
 * and then refuses the click is worse than one that stays quiet.
 */
export function GrowChrome() {
  return (
    <style data-testid="grow-chrome" data-grow-chrome-style>{`
[data-grow-chrome] .${FIELD_CLASS}:hover:not(:disabled) {
  background: var(--pa-grow-fill-hover);
  border-color: var(--pa-grow-border-hover);
}
[data-grow-chrome] .${FIELD_CLASS}:focus-within:not(:disabled),
[data-grow-chrome] .${FIELD_CLASS}:focus:not(:disabled) {
  border-color: var(--pa-grow-border-hover);
}
[data-grow-chrome] .${FIELD_CLASS}:disabled { opacity: 0.45; cursor: not-allowed; }
[data-grow-chrome] .${SUMMARY_CLASS} {
  cursor: pointer;
  border-radius: ${radius.sm}px;
  padding: ${space.xs}px ${space.sm}px;
  margin-left: -${space.sm}px;
  transition: background-color ${duration.fast}ms ${ease.smooth}, color ${duration.fast}ms ${ease.smooth};
}
[data-grow-chrome] .${SUMMARY_CLASS}:hover {
  background: var(--pa-grow-fill-hover);
  color: var(--pa-grow-ink);
}
[data-grow-chrome] .${LINK_CLASS} {
  text-decoration: none;
  border-bottom: 1px solid transparent;
  transition: border-color ${duration.fast}ms ${ease.smooth}, color ${duration.fast}ms ${ease.smooth};
}
[data-grow-chrome] .${LINK_CLASS}:hover {
  border-bottom-color: var(--pa-grow-accent);
}
`}</style>
  );
}

/**
 * The theme half: the custom properties the rules above read, plus the
 * attribute that scopes them. Spread onto the view's root element.
 *
 * Colours come through variables rather than being written into the CSS text so
 * a theme change is a re-render of one style object, not a re-parse of a
 * stylesheet — and so this file names no palette of its own.
 */
export function growChromeVars(colors: ThemeColors): CSSProperties {
  return {
    '--pa-grow-fill-hover': colors.fillHover,
    '--pa-grow-border-hover': colors.borderHi,
    '--pa-grow-ink': colors.text,
    '--pa-grow-accent': colors.cyan,
  } as CSSProperties;
}
