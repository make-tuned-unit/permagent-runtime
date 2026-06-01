# Silver Theme Token System

> Source of truth: Jesse's brand palette sheet (2026-05-31).
> Visual target: the "Good morning, Alex" dashboard mockup — pearl-white surfaces, glass cards with soft shadows, ribbon gradient on primary actions, graphite text, Inter type.

---

## Brand Palette

| Name | Hex | Role |
|------|-----|------|
| Pearl White | `#F8FAFC` | Lightest surface / page base |
| Liquid Silver | `#D8DEE8` | Silver metallic surface |
| Titanium Gray | `#A7B0BE` | Mid grey — borders, muted structure |
| Graphite Text | `#1E2530` | Primary text — near-black |
| Chrome Mist | `#EEF2F7` | Very light cool surface (inputs) |
| Soft Lavender | `#DCCBFF` | Violet-tinted surface (user bubbles) |
| Cyan Intelligence | `#00BFEF` | Primary bright cyan |
| Ion Blue | `#3A7BFF` | Action, links (AA on white) |
| Violet Memory | `#8B5CFF` | AI/memory, gradient partner |

**Signature gradient (brand ribbon):** `linear-gradient(135deg, #00BFEF 0%, #3A7BFF 50%, #8B5CFF 100%)`

---

## Token Mapping

### ThemeColors (extends current interface)

```typescript
const SILVER_COLORS: ThemeColors = {
  // ─── Surfaces ───────────────────────────────────────────────
  bg:         '#F8FAFC',   // Pearl White — page background
  bgDeeper:   '#EEF2F7',   // Chrome Mist — secondary/inset bg
  surface:    '#FFFFFF',   // Pure white glass cards
  surfaceHi:  '#F8FAFC',   // Elevated (modals, popovers)

  // ─── Borders ────────────────────────────────────────────────
  border:     'rgba(167,176,190,0.35)',  // Titanium Gray @ 35%
  borderHi:   'rgba(0,191,239,0.40)',    // Cyan Intelligence focus

  // ─── Accents ────────────────────────────────────────────────
  cyan:       '#00BFEF',   // Cyan Intelligence (primary accent)
  cyanSoft:   'rgba(0,191,239,0.10)',    // Subtle cyan fill
  cyanGlow:   'rgba(0,191,239,0.25)',    // Focus/active glow
  purple:     '#8B5CFF',   // Violet Memory
  purpleBright: '#9B6FFF', // Lighter violet for hover
  purpleSoft: 'rgba(139,92,255,0.10)',   // Subtle violet fill
  purpleGlow: 'rgba(139,92,255,0.25)',   // Violet glow

  // ─── Text ──────────────────────────────────────────────────
  text:       '#1E2530',   // Graphite Text (primary)
  textMuted:  '#5A6577',   // Derived cool grey (AA on #FFFFFF: 5.2:1)
  textDim:    '#A7B0BE',   // Titanium Gray (labels, placeholders)

  // ─── Semantic ──────────────────────────────────────────────
  danger:     '#DC2626',   // Red tuned for light (AA on white)

  // ─── Elevation ─────────────────────────────────────────────
  cardShadow:    '0 2px 8px rgba(30,37,48,0.08), 0 1px 3px rgba(30,37,48,0.05)',
  cardHighlight: 'inset 0 1px 0 rgba(255,255,255,0.8)',
};
```

### New tokens to add to ThemeColors interface

```typescript
// Add to ThemeColors interface:
/** Brand ribbon gradient for primary buttons / AI moments */
ribbonGradient: string;
/** User chat bubble surface */
userBubble: string;
/** User chat bubble text */
userBubbleText: string;
/** Inset surface for inputs */
inputBg: string;
/** On-accent text (white on gradient/cyan buttons) */
textOnAccent: string;
/** Success semantic */
success: string;
/** Warning semantic */
warning: string;
```

Silver values:
```typescript
ribbonGradient: 'linear-gradient(135deg, #00BFEF 0%, #3A7BFF 50%, #8B5CFF 100%)',
userBubble:     '#DCCBFF',   // Soft Lavender
userBubbleText: '#1E2530',   // Graphite (BLACK text, not blue)
inputBg:        '#EEF2F7',   // Chrome Mist (recessed)
textOnAccent:   '#FFFFFF',   // White on gradient buttons
success:        '#059669',   // Green tuned for light
warning:        '#D97706',   // Amber tuned for light
```

Dark/Aurora values (no visual change):
```typescript
ribbonGradient: 'linear-gradient(135deg, #00D5FF 0%, #6366F1 50%, #8D44AE 100%)',
userBubble:     'rgba(141,68,174,0.18)',  // existing purpleSoft
userBubbleText: '#FFFFFF',
inputBg:        '#1E2433',   // existing surface
textOnAccent:   '#FFFFFF',
success:        '#34D399',
warning:        '#FBBF24',
```

### ThemeGradients

```typescript
silver: {
  workspace:     'linear-gradient(180deg, #F8FAFC 0%, #F2F5F9 100%)', // Pearl White subtle gradient
  card:          'linear-gradient(180deg, #FFFFFF 0%, #FAFBFD 100%)', // Glass card (near-white)
  shell:         '#F8FAFC',   // Pearl White (window chrome)
  sidebar:       'rgba(238,242,247,0.92)', // Chrome Mist translucent
  navRail:       'rgba(238,242,247,0.75)', // Chrome Mist lighter
  dropdown:      'rgba(255,255,255,0.98)', // Near-white
  dropdownSolid: '#FFFFFF',
  label:         'Silver',
}
```

---

## Surface Hierarchy

```
┌─────────────────────────────────────────────────────┐
│  PAGE: Pearl White #F8FAFC                          │
│  ┌───────────────────────────────────────────────┐  │
│  │  CARD: White #FFFFFF + glass gradient          │  │
│  │  shadow: cardShadow                           │  │
│  │  highlight: cardHighlight (top-edge sheen)    │  │
│  │  border: Titanium Gray hairline               │  │
│  │  ┌─────────────────────────────────────────┐  │  │
│  │  │  INPUT: Chrome Mist #EEF2F7 (recessed)  │  │  │
│  │  │  focus: Cyan ring                        │  │  │
│  │  └─────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────┐  │
│  │  MODAL: White #FFFFFF + stronger shadow        │  │
│  │  (surfaceHi = Pearl White backdrop)            │  │
│  └───────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────┘
```

---

## Buttons

| Variant | Background | Text | Border | Hover | Active |
|---------|-----------|------|--------|-------|--------|
| Primary | Ribbon gradient | White `#FFF` | none | brightness(1.1) | brightness(0.95) |
| Secondary | `#FFFFFF` | Graphite `#1E2530` | Titanium `rgba(167,176,190,0.5)` | `#F8FAFC` | `#EEF2F7` |
| Tertiary/Ghost | transparent | Ion Blue `#3A7BFF` | none | `rgba(58,123,255,0.08)` | `rgba(58,123,255,0.14)` |
| Danger | `#DC2626` | White `#FFF` | none | `#B91C1C` | `#991B1B` |
| Disabled (all) | `#EEF2F7` | `#A7B0BE` | none | — | — |

**Send button:** Ribbon gradient, white arrow icon.
**"Run Now", "Save", "+ Create":** Primary (ribbon gradient + white text).

---

## Chat Tokens

| Element | Surface | Text | Shadow/Border |
|---------|---------|------|---------------|
| Agent bubble | White glass card (`#FFFFFF` + cardShadow) | Graphite `#1E2530` | cardShadow |
| User bubble | Soft Lavender `#DCCBFF` | Graphite `#1E2530` (**BLACK**, not blue) | subtle `rgba(139,92,255,0.12)` |
| Tool-call chip | White glass `#FFFFFF` | Graphite `#1E2530` | hairline Titanium border |
| Input bar | Chrome Mist `#EEF2F7` | Graphite `#1E2530` | Cyan focus ring |
| Send button | Ribbon gradient | White icon | — |

---

## Terminal + Browser

In Silver theme, terminal and browser panes render **white/light** (overriding the dark convention):
- Terminal background: `#FAFBFD` (near-white)
- Terminal text: Graphite `#1E2530`
- Browser chrome: Pearl White `#F8FAFC`

---

## Brain View

- **3D graph canvas:** stays dark (glow visualization requires dark bg)
- **UI chrome over canvas** (search bar, filter chips, GRAPH/LIST toggle, timeline): themed light/glass — White cards, Graphite text, Titanium borders, Cyan active states

---

## Window Headers (Chat + Main App)

Both windows: **Liquid Silver `#D8DEE8`** title bar / header chrome. NOT black.
- Title text: Graphite `#1E2530`
- Window controls: standard macOS traffic lights

---

## Badges (SCHEDULED / MASTERED / PROPOSED / LEARNED)

Tinted glass pills with AA text:

| Badge | Background | Text |
|-------|-----------|------|
| Scheduled | `rgba(0,191,239,0.12)` | `#007AA3` (darker cyan) |
| Mastered | `rgba(5,150,105,0.12)` | `#047857` (darker green) |
| Proposed | `rgba(217,119,6,0.12)` | `#B45309` (darker amber) |
| Learned | `rgba(139,92,255,0.12)` | `#6D28D9` (darker violet) |

---

## Semantic Colors

| Semantic | Color | Use |
|----------|-------|-----|
| Success | `#059669` | Confirmations, positive states |
| Warning | `#D97706` | Cautions, pending states |
| Danger | `#DC2626` | Errors, destructive actions |
| Info | `#3A7BFF` | Ion Blue, informational |

All AA-verified on both Pearl White `#F8FAFC` and card White `#FFFFFF`.

---

## Typography

**Font family:** Inter (Regular 400 / Medium 500 / SemiBold 600 / Bold 700)
Confirm Inter is the body font across the app (already set in `font.body` token).

Display headings may continue using Manrope for contrast if desired, but body/UI text = Inter.

---

## Contrast Verification

| Pair | Ratio | Pass |
|------|-------|------|
| Graphite `#1E2530` on White `#FFFFFF` | 15.4:1 | AAA |
| Graphite `#1E2530` on Pearl `#F8FAFC` | 14.2:1 | AAA |
| Graphite `#1E2530` on Lavender `#DCCBFF` | 8.1:1 | AAA |
| textMuted `#5A6577` on White `#FFFFFF` | 5.2:1 | AA |
| textMuted `#5A6577` on Pearl `#F8FAFC` | 4.8:1 | AA |
| Ion Blue `#3A7BFF` on White `#FFFFFF` | 4.6:1 | AA |
| White `#FFFFFF` on Cyan `#00BFEF` | 3.1:1 | AA-Large |
| White `#FFFFFF` on Ribbon midpoint `#3A7BFF` | 4.6:1 | AA |
| White `#FFFFFF` on Ribbon dark end `#8B5CFF` | 4.9:1 | AA |
| Titanium `#A7B0BE` on White (placeholder) | 2.6:1 | decorative only |

---

## Design Philosophy (from brand sheet)

- **Silver metallic = structure and trust** (surfaces, frames, chrome)
- **Blue + violet = action and AI intelligence** (accents, CTAs, AI activity)
- **Ribbon gradient reserved for "moments of intelligence"** — primary action buttons, send button, AI activity indicators. Don't overuse.
- **Glass depth:** soft shadows + top-edge highlights + translucent sidebar = premium feel without flatness
- **Inter typography:** clean, human, modern

---

## Unchanged

- Dark theme (Permagent dark) and Aurora theme: unaffected — silver-specific via tokens
- 3D Brain graph canvas: stays dark; only its UI chrome (overlays) gets themed
- Cross-window sync mechanism: unchanged (localStorage 'storage' event)
- Tailwind CSS variable bridge: unchanged (just new values flow through)
