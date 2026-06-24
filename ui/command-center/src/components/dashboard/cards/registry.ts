import type { ComponentType } from 'react';
import { HeroCard } from './HeroCard';
import { StatsCard } from './StatsCard';
import { InFlightCard } from './InFlightCard';
import { RecentCard } from './RecentCard';
import { DecisionsCard } from './DecisionsCard';

export interface CardRegistryEntry {
  component: ComponentType<any>;
  name: string;
  description: string;
  defaultSize: { w: number; h: number };
}

export const CARD_REGISTRY: Record<string, CardRegistryEntry> = {
  hero:      { component: HeroCard,     name: 'Status',          description: 'Agent status and readiness',    defaultSize: { w: 7, h: 4 } },
  decisions: { component: DecisionsCard, name: 'Decisions',      description: 'What your agent needs from you', defaultSize: { w: 5, h: 4 } },
  stats:     { component: StatsCard,    name: 'Stats',           description: 'Sessions today, memory nodes',  defaultSize: { w: 5, h: 4 } },
  in_flight: { component: InFlightCard, name: 'In Flight',       description: 'Goals Henry is actively working',defaultSize: { w: 12, h: 3 } },
  recent:    { component: RecentCard,   name: 'Recent Activity', description: 'Recently completed sessions',   defaultSize: { w: 12, h: 4 } },
};
