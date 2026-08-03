import type { ComponentType } from 'react';
import { HeroCard } from './HeroCard';
import { StatsCard } from './StatsCard';
import { InFlightCard } from './InFlightCard';
import { RecentCard } from './RecentCard';
import { DecisionsCard } from './DecisionsCard';
import { TimelineCard } from './TimelineCard';
import { TodosCard } from './TodosCard';
import { ManifestCard } from './ManifestCard';

/**
 * Dashboard card registration mechanism (issue #182).
 *
 * Two kinds of cards can be registered:
 *
 *  1. First-party **code cards** — a React component bundled with the app,
 *     declared statically in {@link CARD_REGISTRY} below. These receive their
 *     data as props from the Dashboard's `cardDataMap` (they render whatever
 *     the app already fetched via `/api/dashboard`).
 *
 *  2. **Manifest cards** — declarative card definitions the daemon serves from
 *     `GET /api/dashboard/card-types`. A manifest is pure data (no shipped
 *     code): it names a data endpoint and one of a constrained set of layouts,
 *     and the first-party {@link ManifestCard} renderer draws it. This is the
 *     extension point a skill pack uses to contribute a card without shipping
 *     React into the dashboard's security surface.
 *
 * The rendered registry the Dashboard and AddCardPicker consume is the merge of
 * the two (see {@link mergeRegistry}). First-party keys win on collision so a
 * skill can never shadow a built-in card.
 */

/** The constrained set of layouts the first-party {@link ManifestCard} can draw. */
export type CardLayout = 'stat-grid' | 'list' | 'key-value';

/**
 * An optional configuration affordance a manifest card exposes in its empty
 * state — e.g. the weather card asking for a location before it has data.
 * Submitting the input PUTs `{ query }` to `endpoint`, then the card refetches.
 */
export interface CardConfigure {
  endpoint: string;
  label: string;
  placeholder: string;
}

/**
 * The declarative card definition a skill pack (or the daemon's built-ins)
 * ships. Serialized camelCase by the daemon; consumed as-is here.
 */
export interface CardManifest {
  /** Registry key / persisted card `type`, e.g. `"system_stats"`. */
  type: string;
  name: string;
  description: string;
  defaultSize: { w: number; h: number };
  layout: CardLayout;
  /** Endpoint the ManifestCard polls for this card's data. */
  dataEndpoint: string;
  /** Poll interval in seconds (0 / undefined → fetch once, no polling). */
  refreshSeconds?: number;
  /** Provenance shown in the picker — `"built-in"` or a skill pack's name. */
  source: string;
  /** Optional inline setup flow shown when the endpoint reports unconfigured. */
  configure?: CardConfigure;
}

export interface CardRegistryEntry {
  component: ComponentType<any>;
  name: string;
  description: string;
  defaultSize: { w: number; h: number };
  /** Present for manifest cards; drives the ManifestCard renderer. */
  manifest?: CardManifest;
  /** Provenance for the picker's "from …" indicator. Absent ⇒ first-party. */
  source?: string;
}

/**
 * First-party code cards. Adding one here is still the path for a bespoke,
 * component-based card — but it is no longer the *only* path: manifest cards
 * (below) can be added without editing this file at all.
 */
export const CARD_REGISTRY: Record<string, CardRegistryEntry> = {
  hero:      { component: HeroCard,     name: 'Status',          description: 'Agent status and readiness',    defaultSize: { w: 7, h: 4 } },
  decisions: { component: DecisionsCard, name: 'Decisions',      description: 'What your agent needs from you', defaultSize: { w: 5, h: 4 } },
  stats:     { component: StatsCard,    name: 'Stats',           description: 'Sessions today, memory nodes',  defaultSize: { w: 5, h: 4 } },
  in_flight: { component: InFlightCard, name: 'In Flight',       description: 'Goals Henry is actively working',defaultSize: { w: 12, h: 3 } },
  recent:    { component: RecentCard,   name: 'Recent Activity', description: 'Recently completed sessions',   defaultSize: { w: 12, h: 4 } },
  timeline:  { component: TimelineCard, name: 'Timeline',        description: 'What your agents did, day by day', defaultSize: { w: 12, h: 6 } },
  todos:     { component: TodosCard,    name: 'To-dos',          description: 'Dated cards from every board, soonest first', defaultSize: { w: 5, h: 6 } },
};

/** Turn a declarative manifest into a registry entry backed by ManifestCard. */
export function manifestToEntry(manifest: CardManifest): CardRegistryEntry {
  return {
    component: ManifestCard,
    name: manifest.name,
    description: manifest.description,
    defaultSize: manifest.defaultSize,
    manifest,
    source: manifest.source,
  };
}

/**
 * Merge the static first-party registry with the daemon-served manifests.
 * First-party entries win on key collision — a skill pack manifest can never
 * override or impersonate a built-in card type.
 */
export function mergeRegistry(
  base: Record<string, CardRegistryEntry>,
  manifests: CardManifest[],
): Record<string, CardRegistryEntry> {
  const merged: Record<string, CardRegistryEntry> = { ...base };
  for (const manifest of manifests) {
    if (merged[manifest.type]) continue; // built-in wins
    merged[manifest.type] = manifestToEntry(manifest);
  }
  return merged;
}
