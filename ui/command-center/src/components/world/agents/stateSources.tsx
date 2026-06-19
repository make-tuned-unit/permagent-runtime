// Real state sources → shared/agentStatus.ts boundary. WORLD_VIEW_BIBLE.md §6, §9 S1.
//
//   Henry      'daemon'  poll /api/henry/status:
//                          idle → available, in_conversation|tool_call → working,
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
//   Aria/Felix/Nova 'sim' roster from /api/agents; activity is the local wander sim.
//                          The shared clamp holds them to idle/available — LAW (§4).
//                          Daemon AgentStateChanged follow-up (issue #288) lifts this.

import { useEffect } from 'react';
import { api, getApiBaseUrl } from '../../../lib/api';
import { setAgentSource } from '../shared/agentStatus';
import { bankEvent } from '../shared/tendingBank';
import { noteDescribe } from './librarianMining';
import type { AgentHudState } from '../shared/palette';
import { ROSTER } from './roster';

const HENRY_POLL_MS = 2000;
const WS_RETRY_MS = 3000;
const SIM_TOGGLE_MIN_MS = 20000;
const SIM_TOGGLE_MAX_MS = 40000;

function mapHenryState(currentState: string): AgentHudState {
  switch (currentState) {
    case 'in_conversation':
    case 'tool_call':
      return 'working';
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
        const name = s.identity?.name || 'Aria';
        setAgentSource('henry', name, mapHenryState(s.current_state), 'daemon');
      } catch {
        if (!cancelled) setAgentSource('henry', 'Aria', 'error', 'daemon');
      }
    };
    poll();
    const t = setInterval(poll, HENRY_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

  // ── Librarian — /events WebSocket (read-only consumption) ──────────
  useEffect(() => {
    let closed = false;
    let ws: WebSocket | null = null;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const mountedAt = Date.now();

    setAgentSource('librarian', 'The Librarian', 'idle', 'daemon');

    const connect = () => {
      if (closed) return;
      const base = getApiBaseUrl().replace(/^http/, 'ws');
      ws = new WebSocket(`${base}/events`);
      ws.onmessage = (ev) => {
        try {
          const event = JSON.parse(ev.data);
          // Wire shape: { id, type: "librarian_describe_started", timestamp, payload }
          const type: string = event.type ?? event.event_type ?? '';
          // /events replays its buffer on connect — skip pre-mount history so the
          // Librarian stays "idle until first event" AND the tending bank only ever
          // counts work seen live (bible §6; tendingBank honesty note).
          const ts = Date.parse(event.timestamp ?? '');
          const replayed = Number.isFinite(ts) && ts < mountedAt;
          const payload = event.payload ?? {};
          const ref: string =
            payload.memory_key ?? payload.key ?? payload.id ?? event.id ?? '';

          // ── Tending bank: real describe/ingest events deposit material ──
          // (bible §4 — describe = quarried stone, ingest = ore). Live only.
          if (!replayed) {
            if (type === 'librarian_describe_completed') bankEvent('describe', ref, 'daemon');
            else if (type === 'memory_added') bankEvent('ingest', ref, 'daemon');
          }

          // ── Agent runtime state (#288 interim A): live state, flips sim→daemon ──
          // Apply even when replayed: the latest buffered transition IS the current
          // state, and the Henry poll reconciles within 2s either way.
          if (type === 'agent_state_changed') {
            const agentId: string = payload.agent_id ?? '';
            const agentState = payload.state as AgentHudState | undefined;
            if (agentId && agentState) {
              setAgentSource(agentId, payload.name ?? agentId, agentState, 'daemon');
            }
            return;
          }

          if (!type.startsWith('librarian_')) return;
          if (replayed) return;

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
        } catch {
          // ignore malformed events
        }
      };
      ws.onerror = () => ws?.close();
      ws.onclose = () => {
        if (!closed) retry = setTimeout(connect, WS_RETRY_MS);
      };
    };
    connect();

    return () => {
      closed = true;
      if (retry) clearTimeout(retry);
      ws?.close();
    };
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
