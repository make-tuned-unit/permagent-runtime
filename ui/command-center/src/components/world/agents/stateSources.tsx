// Real state sources → shared/agentStatus.ts boundary. WORLD_VIEW_BIBLE.md §6, §9 S1.
//
//   Henry      'daemon'  poll /api/henry/status:
//                          idle → available, in_conversation|tool_call → working,
//                          error → error (#348: real reply-loop failure, latched
//                          in the daemon runtime registry — the poll reports it so
//                          it survives instead of being clobbered back to working),
//                          fetch/parse failure → error (daemon unreachable IS a real signal)
//   Librarian  'daemon'  /events WebSocket (read-only). Wire types are
//                          snake_case (PermagentEventType has
//                          #[serde(rename_all = "snake_case")] and the envelope
//                          renames event_type to "type"):
//                          librarian_describe_started|_retry|_token → working,
//                          librarian_describe_completed → available,
//                          librarian_*error|failed → error, idle until first event.
//                          /events replays its buffer on connect — replayed
//                          history (timestamp before page load) is ignored.
//   Reader     'sim'     roster entry with no live status endpoint yet; activity
//                          is the local wander sim. The shared clamp holds it to
//                          idle/available — LAW (§4). A real reader-event wire is a
//                          follow-up (cf. issue #288 daemon AgentStateChanged).

import { useEffect } from 'react';
import { api } from '../../../lib/api';
import { useCommandCenter } from '../../../lib/store';
import { setAgentSource } from '../shared/agentStatus';
import { subscribeWorldEvents } from '../shared/worldEvents';
import { noteDescribe } from './librarianMining';
import { extractHenryWork, setHenryWork } from './henryWork';
import { noteNudge } from './watcherNudge';
import { noteTaskEvent } from './taskArtifacts';
import type { AgentHudState } from '../shared/palette';
import { ROSTER } from './roster';

const HENRY_POLL_MS = 2000;
const SIM_TOGGLE_MIN_MS = 20000;
const SIM_TOGGLE_MAX_MS = 40000;

function mapHenryState(currentState: string): AgentHudState {
  switch (currentState) {
    case 'in_conversation':
    case 'tool_call':
      return 'working';
    case 'error':
      return 'error';
    case 'idle':
    default:
      return 'available';
  }
}

/** Non-visual: feeds setAgentSource from the real daemon signals + local sim. */
export function AgentStateSources() {
  // ── Henry — daemon status poll ─────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const s = await api.getHenryStatus();
        if (cancelled) return;
        // Fall back to the STORE's agent name, never a literal — "Henry" is
        // this hub's default persona, not the product, and every user names
        // their own agent (`/api/agent/identity`). A literal here renames
        // someone else's agent the moment the identity field is absent.
        const name = s.identity?.name || useCommandCenter.getState().agentName;
        setAgentSource('henry', name, mapHenryState(s.current_state), 'daemon');
        // Same poll, second truth: the REAL in-flight tool name (#84) feeds
        // the HenryToolSigil. One endpoint, no extra network.
        setHenryWork(extractHenryWork(s));
      } catch {
        if (!cancelled) {
          setAgentSource('henry', useCommandCenter.getState().agentName, 'error', 'daemon');
          // Unreachable daemon must not leave a stale tool claim floating.
          setHenryWork({ tool: null, tasksInFlight: 0 });
        }
      }
    };
    poll();
    const t = setInterval(poll, HENRY_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

  // ── Live wire — shared /events socket (shared/worldEvents) ─────────
  // One subscription now carries every event-driven world truth: the
  // Librarian's describe loop, agent_state_changed, task artifacts and the
  // Watcher's nudge. Replay semantics per consumer (worldEvents marks them).
  useEffect(() => {
    setAgentSource('librarian', 'The Librarian', 'idle', 'daemon');

    return subscribeWorldEvents((evt) => {
      const { type, payload, replayed } = evt;

      // ── Agent runtime state (#288 interim A): live state, flips sim→daemon ──
      // Apply even when replayed: the latest buffered transition IS the current
      // state, and the Henry poll reconciles within 2s either way.
      if (type === 'agent_state_changed') {
        const agentId = typeof payload.agent_id === 'string' ? payload.agent_id : '';
        const agentState = payload.state as AgentHudState | undefined;
        if (agentId && agentState) {
          const name = typeof payload.name === 'string' ? payload.name : agentId;
          setAgentSource(agentId, name, agentState, 'daemon');
        }
        return;
      }

      // Everything below animates work — LIVE events only (bible honesty law:
      // the world never re-performs replayed history).
      if (replayed) return;

      // ── Task artifacts: real completions leave stones on the benches ──
      if (type === 'task_completed' || type === 'task_failed') {
        const taskId =
          typeof payload.task_id === 'string' ? payload.task_id : (evt.id ?? String(Date.now()));
        noteTaskEvent(type === 'task_completed' ? 'completed' : 'failed', taskId, Date.now());
        return;
      }

      // ── The Watcher's nudge: beacon flare + delivery walk + plaque ──
      if (type === 'proactive_nudge') {
        noteNudge(payload, Date.now());
        return;
      }

      if (!type.startsWith('librarian_')) return;

      const ref = String(
        payload.memory_key ?? payload.key ?? payload.id ?? evt.id ?? '',
      );

      // ── Librarian mining loop: tablet pull → brighten → reshelve ──
      // (bible §5 Brain: pull a dim tablet, it brightens violet, reshelve glowing).
      if (type === 'librarian_describe_started' || type === 'librarian_describe_retry')
        noteDescribe('start', ref);
      else if (type === 'librarian_describe_completed') noteDescribe('complete', ref);
      else if (/error|failed/.test(type)) noteDescribe('error', ref);

      let next: AgentHudState | null = null;
      if (/error|failed/.test(type)) next = 'error';
      else if (
        type === 'librarian_describe_started' ||
        type === 'librarian_describe_retry' ||
        type === 'librarian_describe_token'
      )
        next = 'working';
      else if (type === 'librarian_describe_completed') next = 'available';
      if (next) setAgentSource('librarian', 'The Librarian', next, 'daemon');
    });
  }, []);

  // ── Sim agents — roster from /api/agents, activity from local sim ──
  useEffect(() => {
    let cancelled = false;
    const simAgents = ROSTER.filter((a) => a.id !== 'henry' && a.id !== 'librarian');
    const names = new Map(simAgents.map((a) => [a.id, a.name]));

    // Roster consumption: pick up display names from the daemon when present.
    api
      .getAgents()
      .then(({ agents }) => {
        if (cancelled) return;
        for (const a of agents) {
          if (names.has(a.id) && a.name) names.set(a.id, a.name);
        }
      })
      .catch(() => {
        // roster unavailable — sim agents remain ambient life with local names
      });

    const timers: ReturnType<typeof setTimeout>[] = [];
    const schedule = (id: string, state: AgentHudState) => {
      setAgentSource(id, names.get(id) ?? id, state, 'sim');
      const delay =
        SIM_TOGGLE_MIN_MS + Math.random() * (SIM_TOGGLE_MAX_MS - SIM_TOGGLE_MIN_MS);
      const t = setTimeout(() => {
        if (cancelled) return;
        schedule(id, state === 'idle' ? 'available' : 'idle');
      }, delay);
      timers.push(t);
    };
    simAgents.forEach((a, i) => schedule(a.id, i % 2 === 0 ? 'idle' : 'available'));

    return () => {
      cancelled = true;
      timers.forEach(clearTimeout);
    };
  }, []);

  return null;
}
