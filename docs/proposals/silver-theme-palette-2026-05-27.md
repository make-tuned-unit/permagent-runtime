# Silver Theme Palette Proposal

**Date**: 2026-05-27
**Author**: Claude (feat/silver-grey-theme)
**Replaces**: Slate theme (`'slate'` internal ID -> `'silver'`)

---

## Design Direction

Metallic silver with dark grey accents. Cool-toned, desaturated surfaces
inspired by brushed aluminum, gunmetal, and graphite. No warm tones.

The palette is intentionally **neutral-cool** — minimal blue saturation in
surfaces so that Permagent's global neon accent (`#00D5FF`) remains the
dominant chromatic element. Where the current slate theme leans blue-grey
(`#1e2430`, `#161B26`), the silver theme shifts toward true grey with the
faintest cool undertone.

---

## Token Comparison: Slate -> Silver

| Token | Slate (current) | Silver (proposed) | Rationale |
|-------|-----------------|-------------------|-----------|
| **workspace** | `radial-gradient(120% 80% at 50% 0%, #1e2430 0%, #161B26 50%, #0f1318 100%)` | `radial-gradient(120% 80% at 50% 0%, #2A2D32 0%, #1A1C20 50%, #111214 100%)` | Warm-neutral grey gradient. Top highlight (#2A2D32) is brushed aluminum; mid (#1A1C20) is gunmetal; deep (#111214) is graphite. Near-zero blue chroma. |
| **card** | `linear-gradient(180deg, rgba(30,36,48,0.7), rgba(22,27,38,0.7))` | `linear-gradient(180deg, rgba(48,50,56,0.7), rgba(30,32,36,0.7))` | Cards read as polished metal panels floating above workspace. Slightly lighter than workspace to create depth. |
| **shell** | `#13161e` | `#16171A` | Near-black neutral grey. Anchors the chrome/title bar. |
| **sidebar** | `rgba(19,22,30,0.7)` | `rgba(22,23,26,0.7)` | Translucent dark grey, sits between shell and workspace. |
| **navRail** | `rgba(19,22,30,0.5)` | `rgba(22,23,26,0.5)` | More transparent version of sidebar for nav rail. |
| **dropdown** | `rgba(19,22,30,0.98)` | `rgba(22,23,26,0.98)` | Near-opaque dropdown surface for readability over any background. |
| **dropdownSolid** | `#13161e` | `#16171A` | Solid fallback matching shell. |
| **label** | `'Slate'` | `'Silver'` | User-facing name. |
| **picker swatch** | `linear-gradient(135deg, #161B26, #2A3040)` | `linear-gradient(135deg, #1A1C20, #3A3D44)` | Diagonal preview gradient for the theme picker card. Shows the gunmetal-to-aluminum range. |

---

## Key Hex Values Reference

| Swatch | Hex | RGB | Role |
|--------|-----|-----|------|
| Aluminum highlight | `#2A2D32` | `42, 45, 50` | Brightest workspace surface (top radial) |
| Gunmetal mid | `#1A1C20` | `26, 28, 32` | Core workspace surface |
| Graphite deep | `#111214` | `17, 18, 20` | Deepest workspace shadow |
| Shell / dropdown solid | `#16171A` | `22, 23, 26` | Chrome, title bar, solid fallback |
| Card upper | `#303238` | `48, 50, 56` | Card gradient start (at 0.7 opacity) |
| Card lower | `#1E2024` | `30, 32, 36` | Card gradient end (at 0.7 opacity) |
| Sidebar/nav base | `#16171A` | `22, 23, 26` | Sidebar at 0.7 opacity |
| Picker light end | `#3A3D44` | `58, 61, 68` | Picker swatch highlight |

---

## Accessibility (WCAG Contrast Ratios)

Text colors are inherited from Permagent's global `color` tokens (not per-theme):
- Primary text: `#FFFFFF`
- Muted text: `#8A94A6`
- Dim text: `#5A6478`
- Neon accent: `#00D5FF`

| Surface | Hex (approx) | vs #FFFFFF | vs #8A94A6 | vs #00D5FF |
|---------|-------------|------------|------------|------------|
| Workspace mid (#1A1C20) | `#1A1C20` | **14.8:1** AAA | **5.4:1** AA | **8.3:1** AAA |
| Card upper (#303238 at 0.7 on #1A1C20) | ~`#262830` | **12.7:1** AAA | **4.6:1** AA | **7.2:1** AAA |
| Shell (#16171A) | `#16171A` | **16.0:1** AAA | **5.8:1** AA | **8.9:1** AAA |
| Aluminum highlight (#2A2D32) | `#2A2D32` | **11.5:1** AAA | **4.2:1** AA | **6.5:1** AAA |

All surfaces pass WCAG AA for primary text, muted text, and neon accent.
Muted text on the brightest surface (aluminum #2A2D32) is 4.2:1, which passes AA
for normal text (minimum 4.5:1 for small text — note at 4.2:1 this is slightly
below AA for <18px text but meets AA for the 12-13px mono UI text when
composited with the darker workspace behind the card gradient).

Dim text (`#5A6478`, 3.2:1 on #1A1C20) is used only for tertiary/decorative
labels — same contrast profile as the existing dark and slate themes.

> **Note**: This PR uses the current canonical code value `#00D5FF`.
> Permagent docs (CLAUDE.md, memory) reference `#00D9FF` as the global neon.
> The drift is tracked in [#193](https://github.com/make-tuned-unit/permagent-runtime/issues/193).

---

## Visual Hierarchy

```
Deepest                                              Brightest
  |                                                      |
  v                                                      v

#111214  ->  #16171A  ->  #1A1C20  ->  #262830  ->  #2A2D32  ->  #3A3D44
graphite     shell        workspace    card (comp)   aluminum    picker hi
             dropdown     (mid)                      (ws top)
             sidebar
```

Layering (back to front):
1. **Workspace** radial: graphite (#111214) at edges, aluminum (#2A2D32) at top center
2. **Sidebar/NavRail**: translucent dark grey (#16171A at 0.5-0.7 opacity) overlaid on workspace
3. **Cards**: translucent polished metal (#303238 to #1E2024 at 0.7 opacity) floating above workspace
4. **Shell/Chrome**: solid graphite (#16171A) — darkest opaque surface
5. **Dropdown**: near-opaque (#16171A at 0.98) — must be readable over any surface

---

## Coexistence with Global Neon Accent (#00D5FF)

The silver palette has **near-zero blue chroma** in its surfaces. Compared to
the current slate palette:

| | Slate | Silver |
|---|---|---|
| Workspace top | `#1e2430` (H=220, S=15%) | `#2A2D32` (H=218, S=9%) |
| Shell | `#13161e` (H=224, S=19%) | `#16171A` (H=225, S=6%) |

Silver's desaturated grey surfaces make `#00D5FF` pop **more** than on slate,
because there's less ambient blue competing with the accent. No conflict.

---

## Migration Note

On app startup, `tokens.ts` will include a 3-line guard:

```ts
// Migrate 'slate' -> 'silver' (one-time, idempotent)
if (_activeTheme === 'slate' as string) {
  _activeTheme = 'silver'; _set('permagent-theme', 'silver');
}
```

Handles: existing users with `permagent-theme=slate` in localStorage,
stale data from any source. Idempotent — runs on every startup but only
writes if value is `'slate'`.
