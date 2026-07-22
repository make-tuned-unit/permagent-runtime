# Dashboard Card Extensibility

Status: **implemented (Phase 2)** — issue #182
Follow-on cards: issue #181

## Problem

Dashboard cards were a hardcoded `Record<string, CardRegistryEntry>` in
`ui/command-center/src/components/dashboard/cards/registry.ts`. Every card was a
first-party React component, so adding a card type meant editing bundled source.
For the skill-pack architecture — where third-party / community skills extend the
agent — this meant a skill pack could never contribute a dashboard card.

## Design decision

We ship **two** registration paths, mirroring the phased plan in #182:

1. **First-party code cards** (unchanged). A bespoke React component declared in
   `CARD_REGISTRY`. This remains the path for cards that need custom visuals
   (hero, stats, in-flight, recent, timeline, decisions). It is no longer the
   *only* path.

2. **Manifest cards** (new — the extension point). A card is described by a
   pure-data **manifest** the daemon serves from `GET /api/dashboard/card-types`.
   The manifest names a data endpoint and one of a constrained set of layouts;
   the first-party `ManifestCard` component renders it. **No skill-provided code
   runs in the dashboard** — the extension surface is a *data* boundary, not a
   *code* boundary. This is Phase 2 of the issue's sequencing and deliberately
   defers Phase 3 (sandboxed component cards) until manifests prove limiting.

The rendered registry the UI consumes is the merge of the two
(`mergeRegistry`). **First-party keys win on collision**, so a skill pack
manifest can never shadow or impersonate a built-in card type.

## The manifest format

```jsonc
{
  "type": "system_stats",                       // registry key & persisted card type
  "name": "System",
  "description": "CPU, memory, and disk at a glance",
  "defaultSize": { "w": 5, "h": 4 },
  "layout": "stat-grid",                        // stat-grid | list | key-value
  "dataEndpoint": "/api/dashboard/system-stats",
  "refreshSeconds": 30,                          // 0/absent ⇒ fetch once
  "source": "built-in",                          // "built-in" or a skill pack name
  "configure": {                                 // optional inline setup flow
    "endpoint": "/api/dashboard/weather/location",
    "label": "Set location",
    "placeholder": "City, e.g. San Francisco"
  }
}
```

### Constrained layouts

`ManifestCard` renders exactly three layouts, each fed the normalized
`CardData` payload (`{ cells: CardCell[], note?, configured? }`):

| layout      | how `CardCell` is drawn                                        |
| ----------- | ------------------------------------------------------------- |
| `stat-grid` | 2-column grid of big-number stats (`label`, `value`, `delta`) |
| `key-value` | rows of `label … value`                                       |
| `list`      | rows of `label` (title) / `sub` (subtitle) / `value` (meta)   |

Keeping the layout set small is the security/consistency win: a manifest can
only *arrange data*, never inject markup or script.

### Data fetching

Every data endpoint returns the same `CardData` shape. `ManifestCard`
self-fetches on mount and polls on `refreshSeconds`. A failed poll keeps the
last good data rather than blanking. `configured: false` (with a `configure`
block on the manifest) renders an inline setup input that PUTs `{ query }` to
the configure endpoint, then refetches — the weather card's location flow.

## Rendering safety

- Manifests are data. The only executor is first-party `ManifestCard`.
- No `dangerouslySetInnerHTML`, no dynamic `import()`, no eval. Values render as
  text nodes.
- Built-in card types win on key collision, so a manifest cannot take over
  `hero`, `stats`, etc.

## Discovery

`AddCardPicker` lists the merged registry. Manifest cards whose `source` is not
`"built-in"` get a small provenance badge (the "from skill X" indicator #182
asked for).

## Versioning (future)

The manifest is intentionally additive-friendly. When the schema evolves,
manifests can carry an optional `minDashboardVersion`; the frontend would skip
manifests it is too old to render. Not needed yet — noted so the format doesn't
have to break later.

## Skill-pack seam (future work)

Today `builtin_card_manifests()` (in `crates/goose-server/src/routes/dashboard_cards.rs`)
returns the daemon's own cards. When a skill-pack registry exists, an installed
pack contributes manifests that are appended to this list, each with
`source: "<pack name>"` and a `dataEndpoint` the pack's own routes serve. Nothing
in the frontend changes — the card appears in the picker automatically. Phase 3
(sandboxed component cards via iframe/shadow-DOM) remains open should a pack need
a layout the constrained set can't express.

## Files

- `ui/command-center/src/components/dashboard/cards/registry.ts` — types +
  `mergeRegistry` / `manifestToEntry`.
- `ui/command-center/src/components/dashboard/cards/ManifestCard.tsx` — the
  generic renderer.
- `ui/command-center/src/components/dashboard/cards/useCardRegistry.ts` — fetch
  manifests + merge.
- `crates/goose-server/src/routes/dashboard_cards.rs` — `GET
  /api/dashboard/card-types` + the #181 built-in manifests and their data
  endpoints.
