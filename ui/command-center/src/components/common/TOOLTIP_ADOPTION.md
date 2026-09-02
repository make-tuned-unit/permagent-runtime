# Adopting the shared `Tooltip` primitive

R3 owns `components/common/Tooltip.tsx`. Other UI DAG lanes convert their own
`title=` attributes — do not edit directories you do not own.

## Recipe

1. Import the wrapper:

```tsx
import { Tooltip } from '../common/Tooltip';
// or from '../../components/common/Tooltip' outside components/
```

2. Replace a native title with a wrapper (strip `title=` from the child):

```tsx
// before
<button title="Collapse" onClick={...}>«</button>

// after
<Tooltip content="Collapse" placement="right">
  <button onClick={...}>«</button>
</Tooltip>
```

3. Prefer `placement` when the trigger sits against an edge (`top` | `bottom` |
   `left` | `right`). The primitive flips at the viewport; sidebar row labels
   that must dodge the native browser webview keep using `SidebarTooltip` /
   `placeSidebarTooltip`.

4. Non-interactive hosts (a `<span>` that only carries an explanation) need
   `tabIndex={0}` so keyboard focus can open the tip.

5. Do **not** convert:
   - `<ViewHeader title>`, `<DetailModal title>`, `<FormModal title>`,
     `<ConfirmDialog title>` — those are headings, not hover tips.
   - `<iframe title>` / `<svg><title>` — required accessible names (allowlisted
     in `Tooltip.test.tsx`).

## Codemod (semi-automated)

From `ui/command-center`:

```bash
node ../../scripts/codemod-title-to-tooltip.mjs path/to/owned/dir
```

The script rewrites simple `title="…"` / `title={'…'}` on JSX hosts into a
`<Tooltip content=…>` wrapper and prints anything too complex to touch by hand
(dynamic expressions spanning multiple lines, spread props, etc.).

## Smoke

```bash
npx vitest run src/components/common/Tooltip.test.tsx
rg -n '\btitle=' src/components/<your-dir> --glob '*.tsx'
```
