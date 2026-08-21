/**
 * The portrait SPEC for Settings → Agents — a pure derivation, no React and no
 * assets. There are no per-character image files on disk: the 3D body is
 * procedural (world/agents/rig.ts), so a portrait has to be derived from the
 * same identity the world uses, or the same agent would wear two different
 * faces in two places.
 *
 * Identity comes STRAIGHT from the world ROSTER. Nothing here invents a colour:
 * an agent with no in-world character reports `trimColor: null` so the renderer
 * can fall back to a neutral token rather than pick a hue that would read as an
 * identity the world never assigned (WORLD_VIEW_BIBLE §2, §4).
 */

import { worldAgentIdForAgent } from '../../../lib/worldAgentIds';
import { ROSTER } from '../../world/agents/roster';

/**
 * The characters the 3D rig builds signature gear for AND the agents API can
 * name. The world roster is larger — the orchestrator and the Reader live
 * in-world with no agents-API entry — and this list is deliberately not: every
 * caller of `portraitSpec` passes a worker id or a dispatch-persona key, so a
 * variant for an id that cannot arrive here would be gear no screen can draw,
 * kept alive only by the test that asserts it exists.
 *
 * Mirrors the reachable half of the list at world/agents/AgentCharacterV2.tsx.
 */
const GEAR_VARIANTS = ['librarian', 'watcher', 'steward', 'strix', 'financier'] as const;

export type PortraitVariant = (typeof GEAR_VARIANTS)[number] | 'unknown';

export interface PortraitSpec {
  /** World roster id, or null when this agent has no in-world character. */
  worldId: string | null;
  variant: PortraitVariant;
  /**
   * Identity trim STRAIGHT from ROSTER. null when there is no character, so
   * there is no identity colour to borrow — the renderer uses a neutral token
   * from the same frozen palette rather than inventing a hue.
   */
  trimColor: string | null;
  /** ROSTER.weathering, 0-1 — the same darkening the 3D body applies. */
  weathering: number;
  label: string;
}

/**
 * Deterministic for a given id: no randomness, no clock, no theme read. Two
 * calls for the same agent must be indistinguishable, because the roster row and
 * the profile header draw the same agent from separate call sites.
 *
 * Resolution goes through the id bridge and NOWHERE else. That bridge is the one
 * thing that knows a dispatch-persona key can name a worker under a second
 * spelling (`steward` in agent.yaml, `git_steward` in the worker registry), and
 * routing every id through it is what stops the Steward drawing its real
 * character on one of its two rows and a blank silhouette on the other.
 */
export function portraitSpec(agentId: string): PortraitSpec {
  const worldId = worldAgentIdForAgent(agentId);
  const identity = worldId === null ? undefined : ROSTER.find(a => a.id === worldId);
  if (!identity) {
    // Not a failure: the API knows more agents than the world does. A neutral
    // silhouette is the honest answer, and it must never borrow another
    // character's trim to look complete.
    return {
      worldId: null,
      variant: 'unknown',
      trimColor: null,
      weathering: 0,
      label: agentId,
    };
  }
  const variant = (GEAR_VARIANTS as readonly string[]).includes(identity.id)
    ? (identity.id as PortraitVariant)
    : 'unknown';
  return {
    worldId: identity.id,
    variant,
    trimColor: identity.trimColor,
    weathering: identity.weathering,
    label: identity.name,
  };
}
