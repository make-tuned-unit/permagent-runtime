/**
 * World W2 — local styles for the place panels, built only from design tokens.
 *
 * No global CSS: every value is a token or a forum CSS custom property with a
 * themed fallback (the panels render inside `.forum-shell`, which defines
 * `--forum-*`; outside it the theme colors apply). Spacing, radius and type
 * come from `styles/tokens.ts`, so the repo-wide scale gates hold.
 */
import type { CSSProperties } from 'react';
import { duration, ease, font, getThemedColors, radius, space, type, type ThemeColors } from '../../../../styles/tokens';

export function placeColors(): ThemeColors {
  return getThemedColors();
}

const v = (name: string, fallback: string) => `var(--forum-${name}, ${fallback})`;

export function placeStyles(colors: ThemeColors, compact: boolean) {
  const text = v('text', colors.text);
  const muted = v('muted', colors.textMuted);
  const border = v('border', colors.border);
  const inset = v('inset', colors.surface);
  const accent = v('accent', colors.cyan);
  return {
    text, muted, border, inset, accent,
    panel: {
      boxSizing: 'border-box', width: '100%', maxWidth: '100%', maxHeight: '100%', overflowY: 'auto',
      padding: compact ? space.xl : space.xxxl, color: text, fontFamily: font.body,
      display: 'flex', flexDirection: 'column', gap: compact ? space.lg : space.xxl,
    } satisfies CSSProperties,
    header: { display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: space.xl } satisfies CSSProperties,
    headerText: { display: 'flex', flexDirection: 'column', gap: space.xxs, minWidth: 0 } satisfies CSSProperties,
    eyebrow: { ...type.label, color: muted, margin: 0 } satisfies CSSProperties,
    title: { ...(compact ? type.heading : type.title), margin: 0 } satisfies CSSProperties,
    hostLine: { ...type.small, color: muted, margin: 0 } satisfies CSSProperties,
    stateLine: { ...type.caption, margin: 0, display: 'flex', alignItems: 'center', gap: space.sm } satisfies CSSProperties,
    dot: (tone: 'ok' | 'stale' | 'error' | 'setup'): CSSProperties => ({
      width: space.sm, height: space.sm, borderRadius: radius.pill, flexShrink: 0,
      background: tone === 'ok' ? colors.success : tone === 'error' ? colors.danger : tone === 'stale' ? colors.warning : colors.textMuted,
    }),
    actions: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.sm } satisfies CSSProperties,
    primaryHint: { ...type.caption, color: muted, margin: 0, flexBasis: '100%' } satisfies CSSProperties,
    body: { display: 'grid', gridTemplateColumns: compact ? 'minmax(0, 1fr)' : 'repeat(2, minmax(0, 1fr))', gap: compact ? space.lg : space.xxl, alignItems: 'start' } satisfies CSSProperties,
    column: { display: 'flex', flexDirection: 'column', gap: compact ? space.lg : space.xl, minWidth: 0 } satisfies CSSProperties,
    slot: { border: `1px solid ${border}`, borderRadius: radius.md, background: inset, padding: compact ? space.lg : space.xl, minWidth: 0 } satisfies CSSProperties,
    slotHead: { display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: space.md, margin: 0 } satisfies CSSProperties,
    slotTitle: { ...type.label, color: muted, margin: 0 } satisfies CSSProperties,
    slotMeta: { ...type.micro, color: muted } satisfies CSSProperties,
    note: { ...type.small, color: muted, margin: 0 } satisfies CSSProperties,
    para: { ...type.small, margin: 0 } satisfies CSSProperties,
    strong: { ...type.small, fontWeight: 600, margin: 0 } satisfies CSSProperties,
    warn: { ...type.small, color: colors.danger, margin: 0 } satisfies CSSProperties,
    list: { listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: space.xs } satisfies CSSProperties,
    row: { display: 'flex', flexDirection: 'column', gap: space.xxs, padding: `${space.sm}px 0`, borderTop: `1px solid ${border}` } satisfies CSSProperties,
    option: (selected: boolean): CSSProperties => ({
      display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: space.xxs, width: '100%', textAlign: 'left',
      '--pa-btn-bg': selected ? colors.cyanSoft : 'transparent',
      '--pa-btn-fg': text,
      '--pa-btn-border': selected ? accent : 'transparent',
      '--pa-btn-bg-hover': colors.surfaceHi,
      '--pa-btn-border-hover': border,
      '--pa-btn-bg-active': colors.surface,
      '--pa-btn-radius': `${radius.sm}px`,
      '--pa-btn-pad': `${space.sm}px ${space.md}px`,
    } as CSSProperties),
    optionMeta: { ...type.micro, color: muted } satisfies CSSProperties,
    badge: (tone: 'neutral' | 'warn' | 'danger' | 'ok'): CSSProperties => ({
      ...type.micro, display: 'inline-block', padding: `${space.xxs}px ${space.sm}px`, borderRadius: radius.pill,
      border: `1px solid ${tone === 'danger' ? colors.danger : tone === 'warn' ? colors.warning : tone === 'ok' ? colors.success : border}`,
      color: tone === 'danger' ? colors.danger : tone === 'warn' ? colors.warning : tone === 'ok' ? colors.success : muted,
    }),
    field: { display: 'flex', flexDirection: 'column', gap: space.xs } satisfies CSSProperties,
    label: { ...type.caption, color: muted } satisfies CSSProperties,
    input: {
      ...type.small, boxSizing: 'border-box', width: '100%', padding: `${space.sm}px ${space.md}px`, borderRadius: radius.sm,
      border: `1px solid ${border}`, background: 'transparent', color: text, fontFamily: font.body,
    } satisfies CSSProperties,
    layer: { border: `1px solid ${accent}`, borderRadius: radius.md, padding: compact ? space.lg : space.xl, display: 'flex', flexDirection: 'column', gap: space.md } satisfies CSSProperties,
    preview: { ...type.caption, fontFamily: font.mono, whiteSpace: 'pre-wrap', margin: 0, padding: space.md, borderRadius: radius.sm, border: `1px solid ${border}`, maxHeight: compact ? 120 : 220, overflowY: 'auto' } satisfies CSSProperties,
    facts: { display: 'grid', gridTemplateColumns: compact ? 'repeat(2, minmax(0, 1fr))' : 'repeat(4, minmax(0, 1fr))', gap: space.md, margin: 0 } satisfies CSSProperties,
    fact: { display: 'flex', flexDirection: 'column', gap: space.xxs, minWidth: 0, margin: 0 } satisfies CSSProperties,
    /** Panel entrance: 320 ms spring with a short rise; reduced motion fades only. */
    enter: (shown: boolean, reduced: boolean): CSSProperties => (reduced
      ? { opacity: shown ? 1 : 0, transition: `opacity ${duration.fast}ms ${ease.out}` }
      : { opacity: shown ? 1 : 0, transform: shown ? 'none' : `translateY(${space.md}px)`, transition: `opacity ${duration.slow}ms ${ease.smooth}, transform ${duration.slow}ms ${ease.smooth}` }),
  };
}

export type PlaceStyles = ReturnType<typeof placeStyles>;
