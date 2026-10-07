import type { ComponentType } from 'react';
import { StatsCard } from './StatsCard';
import { InFlightCard } from './InFlightCard';
import { RecentCard } from './RecentCard';
import { DecisionsCard } from './DecisionsCard';
import { TimelineCard } from './TimelineCard';
import { TodosCard } from './TodosCard';
import { ManifestCard } from './ManifestCard';
import { GrowthResultsCard } from './GrowthResultsCard';
import { CouncilCard } from './CouncilCard';
import { TodayFocusCard } from './TodayFocusCard';
import { OvernightDigestCard } from './OvernightDigestCard';
import { FleetKpisCard } from './FleetKpisCard';
import { StalledPatternCard } from './StalledPatternCard';
import { ProjectHealthGridCard } from './ProjectHealthGridCard';
import { CalendarCard } from './CalendarCard';

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

/**
 * The constrained set of layouts the first-party {@link ManifestCard} can draw.
 *
 * `compact` is the small-tile layout: one hero value with the rest of the cells
 * as quiet label/value rows beneath it. Ambient readouts (weather, machine
 * load) use it — they are glanced at, not read, and a full-size card spends
 * dashboard real estate that the things you actually act on should get.
 */
export type CardLayout = 'stat-grid' | 'list' | 'key-value' | 'compact';

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
  /** `true` for a card whose content is naturally short and doesn't grow (a
   *  one-line status strip) — drawn at its content's height, up to
   *  `defaultSize.h`, instead of stretching to fill a reserved grid row
   *  (dagK Phase-1 presentation pass: "card height must follow content"). */
  autoHeight?: boolean;
}

/** A named size a card can be dropped into (grid units: 12 columns, 60px rows). */
export interface CardSizePreset {
  id: string;
  label: string;
  w: number;
  h: number;
}

export interface CardRegistryEntry {
  component: ComponentType<any>;
  name: string;
  description: string;
  defaultSize: { w: number; h: number };
  /** Sizes the Layouts gallery / Customize can offer, smallest first. A card
   *  that draws differently by size (Calendar) declares where it changes. */
  sizePresets?: CardSizePreset[];
  /** Present for manifest cards; drives the ManifestCard renderer. */
  manifest?: CardManifest;
  /** Provenance for the picker's "from …" indicator. Absent ⇒ first-party. */
  source?: string;
}

/**
 * Calendar sizes. Compact (4 columns, ~330px at the usual Home width) is
 * "Next up" plus today's list; Medium (7) and Large (12) are the full
 * Day / Week / Month calendar. The card switches on its measured width
 * (`COMPACT_BELOW_PX` in CalendarCard.tsx), which these presets straddle.
 */
export const CALENDAR_SIZE_PRESETS: CardSizePreset[] = [
  { id: 'compact', label: 'Compact — next up and today', w: 4, h: 4 },
  { id: 'medium', label: 'Medium — full calendar', w: 7, h: 8 },
  { id: 'large', label: 'Large — full calendar, full width', w: 12, h: 9 },
];

/**
 * First-party code cards. Adding one here is still the path for a bespoke,
 * component-based card — but it is no longer the *only* path: manifest cards
 * (below) can be added without editing this file at all.
 */
export const CARD_REGISTRY: Record<string, CardRegistryEntry> = {
  // 'hero' (Status/readiness) retired 2026-09-01 — replaced by the sidebar's
  // nav status indicator beside Home (components/sidebar/NavAgentStatus.tsx).
  // useLayout.ts strips any persisted 'hero' card so an old layout degrades
  // to "not there" instead of a MissingCard ghost tile.
  decisions: { component: DecisionsCard, name: 'Decisions',      description: 'What your agent needs from you', defaultSize: { w: 5, h: 4 } },
  stats:     { component: StatsCard,    name: 'Stats',           description: 'Sessions today, memory nodes',  defaultSize: { w: 5, h: 4 } },
  in_flight: { component: InFlightCard, name: 'In Flight',       description: 'Goals your agent is actively working',defaultSize: { w: 12, h: 3 } },
  recent:    { component: RecentCard,   name: 'Recent Activity', description: 'Recently completed sessions',   defaultSize: { w: 12, h: 4 } },
  timeline:  { component: TimelineCard, name: 'Timeline',        description: 'What your agents did, day by day', defaultSize: { w: 12, h: 6 } },
  todos:     { component: TodosCard,    name: 'To-dos',          description: 'Dated cards from every board, soonest first', defaultSize: { w: 5, h: 6 } },
  growth_results: { component: GrowthResultsCard, name: 'Growth', description: 'Growth actions across all projects — trendline and per-project performance', defaultSize: { w: 12, h: 6 } },
  council: { component: CouncilCard, name: 'Council', description: 'Weekly debate across your connected models, with actions in the Decision Inbox', defaultSize: { w: 12, h: 6 } },
  // Calendar is a first-party code card (not the daemon's manifest): it needs
  // its own state — view, period, popover — and merge order puts this entry
  // ahead of the manifest of the same type, so existing layouts pick it up.
  calendar: { component: CalendarCard, name: 'Calendar', description: 'Day, week and month calendar with your next event up top', defaultSize: { w: 7, h: 8 }, sizePresets: CALENDAR_SIZE_PRESETS },
  // dagK home-templates lane — Morning Briefing + Operator Command Center's
  // new synthesis cards (HOME_TEMPLATES_RESEARCH.md §3). henry_status and
  // job_health are daemon-served MANIFEST cards instead (see
  // dashboard_cards.rs's builtin_card_manifests) — they need no entry here.
  today_focus: { component: TodayFocusCard, name: "Today's Focus", description: 'The one thing today needs from you, and why', defaultSize: { w: 7, h: 4 } },
  overnight_digest: { component: OvernightDigestCard, name: 'Overnight', description: 'What happened while you were away', defaultSize: { w: 3, h: 4 } },
  fleet_kpis: { component: FleetKpisCard, name: 'Fleet KPIs', description: 'Projects, active goals, stalled goals and decisions at a glance', defaultSize: { w: 12, h: 2 } },
  stalled_pattern: { component: StalledPatternCard, name: 'Shared blockers', description: 'Stalled goals across projects that share the same blocker', defaultSize: { w: 12, h: 3 } },
  project_health_grid: { component: ProjectHealthGridCard, name: 'Project health', description: 'One row per project — active goals, held goals, status', defaultSize: { w: 12, h: 5 } },
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
