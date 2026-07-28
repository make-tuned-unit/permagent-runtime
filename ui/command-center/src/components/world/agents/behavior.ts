// Behavior orchestration — binds HUD states (shared/agentStatus) to locomotion +
// engagement. WORLD_VIEW_BIBLE.md §4: working agents walk to a claimed W2 anchor
// and sit; Henry additionally strolls through the Forum Antechamber threshold.
// Straight-line + ease locomotion only — NO pathfinding engine (scope fence).

import { useEffect, useRef } from 'react';
import type { AgentRuntimeState } from '../shared/agentStatus';
import { releaseAgentAnchors } from '../shared/anchors';
import { STATIONS } from '../constants';
import { ROSTER, MEZZ_RADIUS, MEZZ_Y, getIdentity, type AgentIdentity } from './roster';
import { ensureMotion, getAgentPosition, getMotion, setEngaged, setPath, stopAgent, type Waypoint } from './motion';
import { ensurePlaceholderAnchors } from './placeholderAnchors';
import { getNudge } from './watcherNudge';
import { STAIR, stairPointAt } from '../areas/hall/MezzanineLibrary';
import { DAIS, BEAM_MS, setDaisPresence, triggerDaisBeam } from './daisBus';
import { useCommandCenter } from '../../../lib/store';

const WANDER_MIN_MS = 15000;
const WANDER_MAX_MS = 30000;
const HENRY_STROLL_MIN_MS = 60000;
const HENRY_STROLL_MAX_MS = 120000;

// PLACEHOLDER until W1's Antechamber lands (note for rebase): the NW-diagonal
// threshold sits on the colonnade line at r≈15; the pause point stays on the
// platform (r≈19). After W1 lands, extend the path into the room interior.
// Kept INSIDE the rotunda edge (radius 15) so Henry's stroll stays on the surface.
const ANTECHAMBER_THRESHOLD = { x: -9.2, z: -9.2 };
const ANTECHAMBER_PAUSE = { x: -9.9, z: -9.9 };

// A full round trip up the spiral stair to the mezzanine and back down — the climb
// waypoints carry their Y (motion eases toward it), so the agent walks UP the steps.
function stairRoundTrip(): Waypoint[] {
  const N = 8;
  const up: Waypoint[] = [];
  const down: Waypoint[] = [];
  for (let i = 0; i <= N; i++) up.push(stairPointAt(i / N));
  for (let i = 0; i <= N; i++) down.push(stairPointAt(1 - i / N));
  const onRing = (a: number, pauseMs: number): Waypoint => ({
    x: Math.cos(a) * STAIR.radius,
    y: STAIR.height,
    z: Math.sin(a) * STAIR.radius,
    pauseMs,
  });
  return [
    ...up, // ground → mezzanine
    onRing(STAIR.gapCenter + 0.5, 3000),
    onRing(STAIR.gapCenter + 1.1, 3500),
    stairPointAt(1), // back to the stair head
    ...down, // mezzanine → ground
  ];
}

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

function engageForWork(agent: AgentIdentity): void {
  const m = getMotion(agent.id);
  if (!m || m.engaged !== 'none') return;
  // Agents don't sit at desks (2026-07-28 ruling): a workstation is a human
  // metaphor. After the dais transmits the task, the agent processes standing
  // wherever it is — the work halo (rig.workHalo) orbits it while it thinks.
  // Desks and seats remain scenery; the seat-claiming path was removed.
  stopAgent(agent.id);
  setEngaged(agent.id, 'standing');
}

function disengage(agent: AgentIdentity): void {
  releaseAgentAnchors(agent.id);
  setEngaged(agent.id, 'none');
  if (!agent.mezzanineLocked) {
    const t = pickWanderTarget(agent);
    setPath(agent.id, [t]);
  }
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
  const nudgeSeqRef = useRef(0);

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

    // Task pickup choreography: a ground agent whose state flips to `working`
    // first steps ONTO the central dais; on arrival the beam fires and the
    // work transmits down into it (TaskDais); after the beam it steps off and
    // engages at its seat. Guarded at every async step against the state
    // having moved on (work finished / errored mid-walk).
    const summonToDais = (agent: AgentIdentity) => {
      const a = Math.random() * Math.PI * 2;
      const sx = DAIS.x + Math.cos(a) * 0.7;
      const sz = DAIS.z + Math.sin(a) * 0.7;
      setPath(
        agent.id,
        [{ x: sx, y: DAIS.topY, z: sz, facing: Math.atan2(DAIS.x - sx, DAIS.z - sz) }],
        () => {
          if (hudRef.current.get(agent.id) !== 'working') return;
          triggerDaisBeam(agent.id);
          window.setTimeout(() => {
            if (hudRef.current.get(agent.id) !== 'working') return;
            // Step off radially, past the step ring.
            const offX = DAIS.x + ((sx - DAIS.x) / 0.7) * (DAIS.radius + 1.6);
            const offZ = DAIS.z + ((sz - DAIS.z) / 0.7) * (DAIS.radius + 1.6);
            setPath(agent.id, [{ x: offX, y: 0, z: offZ }], () => {
              if (hudRef.current.get(agent.id) === 'working') engageForWork(agent);
            });
          }, BEAM_MS);
        },
      );
    };

    for (const s of states) {
      const before = prev.get(s.id);
      if (before === s.hudState) continue;
      prev.set(s.id, s.hudState);
      const agent = getIdentity(s.id);
      if (!agent) continue;

      if (s.hudState === 'working') {
        // The Librarian is ring-locked on the mezzanine — no ground dais trip.
        if (agent.mezzanineLocked) engageForWork(agent);
        else summonToDais(agent);
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

      // ── The Watcher delivers a REAL nudge (§4-honest choreography) ──
      // A live proactive_nudge arrived: the Watcher carries it to the sovereign —
      // walks to just short of wherever Henry stands right now, presents for a
      // few seconds facing him, then returns to its vigil. Walking claims no
      // HUD state (locomotion is ambient); the plaque + beacon flare that carry
      // the CONTENT read the same real event from watcherNudge.
      const nudge = getNudge();
      if (nudge.seq > 0 && nudge.seq !== nudgeSeqRef.current) {
        nudgeSeqRef.current = nudge.seq;
        const watcher = getMotion('watcher');
        const henry = getAgentPosition('henry');
        const identity = getIdentity('watcher');
        const hud = hudRef.current.get('watcher') ?? 'idle';
        if (watcher && henry && identity && watcher.engaged === 'none' && hud !== 'error') {
          // Approach point ~1.8u from Henry, on the Watcher's side of him.
          const dx = watcher.x - henry.x;
          const dz = watcher.z - henry.z;
          const d = Math.sqrt(dx * dx + dz * dz) || 1;
          const ax = henry.x + (dx / d) * 1.8;
          const az = henry.z + (dz / d) * 1.8;
          wanderAtRef.current.set('watcher', now + 30000);
          setPath('watcher', [
            {
              x: ax,
              y: 0,
              z: az,
              // Face Henry while presenting (motion heading convention: atan2(dx, dz)).
              facing: Math.atan2(henry.x - ax, henry.z - az),
              pauseMs: 8000,
            },
            { x: identity.home.x, y: 0, z: identity.home.z },
          ]);
        }
      }

      // ── Henry beams up during an open conversation ──
      // While the chat dock is open Henry stands ON the dais under the
      // sustained beam — present to the user for the whole conversation.
      // Wander and the Antechamber stroll are suppressed; when the dock
      // closes he steps down and normal life resumes.
      const chatOpen = useCommandCenter.getState().chatDockOpen;
      const henryM = getMotion('henry');
      if (chatOpen) {
        setDaisPresence(true);
        if (henryM && !henryM.walking && henryM.engaged === 'none' && henryM.queue.length === 0) {
          const dx = henryM.x - DAIS.x;
          const dz = henryM.z - DAIS.z;
          if (dx * dx + dz * dz > 0.35 * 0.35 || henryM.y < DAIS.topY - 0.05) {
            setPath('henry', [{ x: DAIS.x, y: DAIS.topY, z: DAIS.z, facing: Math.PI }]);
          }
        }
      } else {
        setDaisPresence(false);
        if (henryM && !henryM.walking && henryM.engaged === 'none' && henryM.queue.length === 0 && henryM.y > 0.1) {
          // Step down off the dais.
          setPath('henry', [{ x: DAIS.radius + 1.6, y: 0, z: 1.5 }]);
        }
      }

      for (const agent of ROSTER) {
        const m = getMotion(agent.id);
        if (!m) continue;
        const hud = hudRef.current.get(agent.id) ?? 'idle';

        if (m.walking || m.engaged !== 'none' || m.queue.length > 0) continue;
        if (hud === 'working' || hud === 'error') continue;
        // Conversation presence pins Henry to the dais — no wander, no stroll.
        if (agent.isHenry && chatOpen) continue;

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
          // Sometimes a ground worker chooses to climb the stairs to the mezzanine
          // and back; otherwise it wanders the floor. (Henry presides; Librarian is
          // ring-locked up top already.)
          if (!agent.isHenry && !agent.mezzanineLocked && Math.random() < 0.3) {
            setPath(agent.id, stairRoundTrip());
          } else {
            setPath(agent.id, [pickWanderTarget(agent)]);
          }
        }
      }
    }, 1000);
    return () => clearInterval(tick);
  }, []);
}
