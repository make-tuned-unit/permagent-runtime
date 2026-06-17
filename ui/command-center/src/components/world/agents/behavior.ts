// Behavior orchestration — binds HUD states (shared/agentStatus) to locomotion +
// engagement. WORLD_VIEW_BIBLE.md §4: working agents walk to a claimed W2 anchor
// and sit; Henry additionally strolls through the Forum Antechamber threshold.
// Straight-line + ease locomotion only — NO pathfinding engine (scope fence).

import { useEffect, useRef } from 'react';
import type { AgentRuntimeState } from '../shared/agentStatus';
import {
  claimAnchor,
  getAnchors,
  releaseAgentAnchors,
  type AgentAnchor,
} from '../shared/anchors';
import { STATIONS } from '../constants';
import { ROSTER, MEZZ_RADIUS, MEZZ_Y, getIdentity, type AgentIdentity } from './roster';
import { ensureMotion, getMotion, setEngaged, setPath, stopAgent } from './motion';
import { ensurePlaceholderAnchors } from './placeholderAnchors';
import { BUILD_ANCHOR_IDS } from './construction';
import { hasBankedWork } from '../shared/tendingBank';

const WANDER_MIN_MS = 15000;
const WANDER_MAX_MS = 30000;
const HENRY_STROLL_MIN_MS = 60000;
const HENRY_STROLL_MAX_MS = 120000;

// PLACEHOLDER until W1's Antechamber lands (note for rebase): the NW-diagonal
// threshold sits on the colonnade line at r≈15; the pause point stays on the
// platform (r≈19). After W1 lands, extend the path into the room interior.
const ANTECHAMBER_THRESHOLD = { x: -10.6, z: -10.6 };
const ANTECHAMBER_PAUSE = { x: -13.4, z: -13.4 };

const MEZZ_WAYPOINTS = Array.from({ length: 12 }, (_, i) => {
  const angle = (i / 12) * Math.PI * 2;
  return { x: Math.cos(angle) * MEZZ_RADIUS, z: Math.sin(angle) * MEZZ_RADIUS };
});

function rand(min: number, max: number): number {
  return min + Math.random() * (max - min);
}

function pickWanderTarget(agent: AgentIdentity): { x: number; y: number; z: number } {
  if (agent.mezzanineLocked) {
    const wp = MEZZ_WAYPOINTS[Math.floor(Math.random() * MEZZ_WAYPOINTS.length)];
    return { x: wp.x, y: MEZZ_Y, z: wp.z };
  }
  // Stations + hall center, with a little scatter so agents don't stack.
  const idx = Math.floor(Math.random() * (STATIONS.length + 1));
  const base =
    idx === STATIONS.length
      ? { x: 0, z: 0 }
      : { x: STATIONS[idx].position[0], z: STATIONS[idx].position[2] };
  return { x: base.x + rand(-1.5, 1.5), y: 0, z: base.z + rand(-1.5, 1.5) };
}

function pickSeatAnchor(agentId: string, from: { x: number; z: number }): AgentAnchor | null {
  const seats = getAnchors()
    .filter((a) => a.kind === 'seat')
    .sort((a, b) => {
      const da = (a.position[0] - from.x) ** 2 + (a.position[2] - from.z) ** 2;
      const db = (b.position[0] - from.x) ** 2 + (b.position[2] - from.z) ** 2;
      return da - db;
    });
  for (const seat of seats) {
    if (claimAnchor(seat.id, agentId)) return seat;
  }
  return null;
}

function engageForWork(agent: AgentIdentity): void {
  const m = getMotion(agent.id);
  if (!m || m.engaged !== 'none') return;
  if (agent.mezzanineLocked) {
    // No seat anchors on the mezzanine — engage standing in place.
    stopAgent(agent.id);
    setEngaged(agent.id, 'standing');
    return;
  }
  const seat = pickSeatAnchor(agent.id, m);
  if (!seat) {
    stopAgent(agent.id);
    setEngaged(agent.id, 'standing');
    return;
  }
  setPath(
    agent.id,
    [{ x: seat.position[0], y: seat.position[1], z: seat.position[2], facing: seat.facing }],
    () => setEngaged(agent.id, 'seated'),
  );
}

function disengage(agent: AgentIdentity): void {
  releaseAgentAnchors(agent.id);
  setEngaged(agent.id, 'none');
  if (!agent.mezzanineLocked) {
    const t = pickWanderTarget(agent);
    setPath(agent.id, [t]);
  }
}

// ── Tending (bible §4): between tasks, idle workers tend the construction sites —
// hauling, setting stones. Gray-warm, unhurried, driven ONLY by banked describe/ingest
// events (hasBankedWork). No bank ⇒ they stay idle, NOT tending. Henry never tends — he
// presides (handled in the tick). The Librarian mines on the mezzanine, not here.
//
// Eligible agents claim one of W2's frozen construction build anchors (the five
// `${mount.id}.build` 'stand' ids) and walk to it; on arrival they enter the 'tending'
// engagement. The AgentCharacterV2 reads engagement + bank to show the tending register.
// DEFENSIVE: those anchors register only when the build area chunk is mounted — if none
// are present yet, pickTendAnchor returns null and the worker stays idle/wanders.

const BUILD_ANCHOR_SET = new Set<string>(BUILD_ANCHOR_IDS);

function pickTendAnchor(agentId: string, from: { x: number; z: number }): AgentAnchor | null {
  const stands = getAnchors()
    .filter((a) => a.kind === 'stand' && BUILD_ANCHOR_SET.has(a.id))
    .sort((a, b) => {
      const da = (a.position[0] - from.x) ** 2 + (a.position[2] - from.z) ** 2;
      const db = (b.position[0] - from.x) ** 2 + (b.position[2] - from.z) ** 2;
      return da - db;
    });
  for (const s of stands) {
    if (claimAnchor(s.id, agentId)) return s;
  }
  return null;
}

function startTending(agent: AgentIdentity): boolean {
  const m = getMotion(agent.id);
  if (!m) return false;
  const anchor = pickTendAnchor(agent.id, m);
  if (!anchor) return false;
  setPath(
    agent.id,
    [{ x: anchor.position[0], y: anchor.position[1], z: anchor.position[2], facing: anchor.facing }],
    () => setEngaged(agent.id, 'tending'),
  );
  return true;
}

function stopTending(agent: AgentIdentity): void {
  releaseAgentAnchors(agent.id);
  setEngaged(agent.id, 'none');
}

/**
 * Drives wander + work engagement from the runtime states. Continuous motion is
 * advanced separately (advanceMotion in WorldAgents' useFrame); this hook only
 * reacts to discrete changes and slow timers.
 */
export function useAgentBehavior(states: AgentRuntimeState[]): void {
  const hudRef = useRef(new Map<string, string>());
  const prevRef = useRef(new Map<string, string>());
  const wanderAtRef = useRef(new Map<string, number>());
  const henryStrollAtRef = useRef(Date.now() + 30000);

  // Spawn motion records + placeholder seat anchors once. Construction build anchors are
  // published by W2's props on area mount — W3 only consumes them (no registration here).
  useEffect(() => {
    ensurePlaceholderAnchors();
    for (const a of ROSTER) {
      ensureMotion(a.id, a.home, a.mezzanineLocked ? MEZZ_RADIUS : null);
    }
  }, []);

  // React to discrete HUD-state changes.
  useEffect(() => {
    const hud = hudRef.current;
    const prev = prevRef.current;
    hud.clear();
    for (const s of states) hud.set(s.id, s.hudState);

    for (const s of states) {
      const before = prev.get(s.id);
      if (before === s.hudState) continue;
      prev.set(s.id, s.hudState);
      const agent = getIdentity(s.id);
      if (!agent) continue;

      if (s.hudState === 'working') {
        engageForWork(agent);
      } else if (before === 'working') {
        disengage(agent);
      }
      if (s.hudState === 'error') {
        // The slump happens where the agent stands — stop and release any claim.
        releaseAgentAnchors(s.id);
        setEngaged(s.id, 'none');
        stopAgent(s.id);
      }
    }
  }, [states]);

  // Slow wander scheduler (1s tick — discrete, not per-frame).
  useEffect(() => {
    const tick = setInterval(() => {
      const now = Date.now();
      const banked = hasBankedWork();
      for (const agent of ROSTER) {
        const m = getMotion(agent.id);
        if (!m) continue;
        const hud = hudRef.current.get(agent.id) ?? 'idle';

        // ── Tending lifecycle (bible §4) — only while real material is banked ──
        // Henry presides (never tends); the Librarian mines the mezzanine, not the
        // construction sites. Everyone else tends when there's banked work and they're
        // otherwise idle. When the bank empties, tenders stand down and resume wander.
        const canTend = !agent.isHenry && !agent.mezzanineLocked;
        if (m.engaged === 'tending') {
          if (!banked || hud === 'working' || hud === 'error') stopTending(agent);
          else continue; // keep tending while material remains
        }
        if (m.walking || m.engaged !== 'none' || m.queue.length > 0) continue;
        if (hud === 'working' || hud === 'error') continue;
        if (canTend && banked) {
          if (startTending(agent)) continue;
        }

        // Henry's Antechamber stroll — through the threshold and back (§4).
        if (agent.isHenry && now >= henryStrollAtRef.current) {
          henryStrollAtRef.current = now + rand(HENRY_STROLL_MIN_MS, HENRY_STROLL_MAX_MS);
          wanderAtRef.current.set(agent.id, now + 30000);
          setPath(agent.id, [
            { x: ANTECHAMBER_THRESHOLD.x, y: 0, z: ANTECHAMBER_THRESHOLD.z, pauseMs: 1200 },
            { x: ANTECHAMBER_PAUSE.x, y: 0, z: ANTECHAMBER_PAUSE.z, pauseMs: 2500 },
            { x: ANTECHAMBER_THRESHOLD.x, y: 0, z: ANTECHAMBER_THRESHOLD.z },
            { x: rand(-2, 2), y: 0, z: rand(-2, 2) },
          ]);
          continue;
        }

        let at = wanderAtRef.current.get(agent.id);
        if (at === undefined) {
          at = now + rand(3000, 12000);
          wanderAtRef.current.set(agent.id, at);
        }
        if (now >= at) {
          wanderAtRef.current.set(agent.id, now + rand(WANDER_MIN_MS, WANDER_MAX_MS));
          setPath(agent.id, [pickWanderTarget(agent)]);
        }
      }
    }, 1000);
    return () => clearInterval(tick);
  }, []);
}
