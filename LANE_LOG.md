# LANE_LOG — R14 browser chrome

## Dispatched
2026-09-02 13:22 — Cursor Agent on `feat/r14-browser-chrome` (worktree already at origin/main).

## Read
- `UI_DAG_RULES.md` (all 8 gates)
- `APPLE_LIQUID_GLASS_RESEARCH.md` §§1.5, 1.6, 3.3, 4, 5 (from main checkout untracked copy)
- `tokens.ts` (THEME_GLASS / getThemedGlass, radius.glass=9, concentric, space, shadow, SPRING_LINEAR / ease / duration)
- `reduceTransparency.ts` (bridge; honoured via `useGlass`)

## Done
- Top chrome (tabs + URL + bookmarks) is **one** `useGlass('glass')` plane; status bar is a second plane below opaque content (D1/D3).
- Children use `fillHover` / `fillActive` via `chromeBareVars` — no glass-on-glass (D2).
- Address field is opaque `inputBg` inset; saved-tabs menu is opaque `surface` + `elevationOverlay` (D2).
- Hardcoded `rgba(...)` / Tailwind red-amber / `shadow-lg` removed from owned chrome files.
- Concentric chip radius under address field (`CHIP_RADIUS`); geometry frozen in `CHROME_GEOM`.
- `syncBounds` body hash unchanged (`ba53557a…0e151e`); content still `flex-1 min-h-0` `containerRef`.
- Kept native `title=` tooltips — no shared Tooltip in `components/common` yet (request R3).
- Tests: `browserChrome.test.ts` + existing browser suite green (109).
- `npx tsc --noEmit` clean. No eslint config/deps in `ui/command-center` (noted in PR).

## Screenshots
Not taken — in-app browser chrome needs the Tauri shell (native webview compositing). Vite-only cannot show glass over live page content or theme-switch the shell.

## Requests
- **R3**: shared tooltip primitive in `components/common`; browser chrome will adopt.
- **A1c / tokens**: optional `textSize` step for dense chrome caption `10` (currently local `CHROME_CAPTION`); half-step `space` for chip pad `2`.
