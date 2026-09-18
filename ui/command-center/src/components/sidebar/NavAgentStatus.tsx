/**
 * The nav-rail status indicator beside Home (2026-09-01 ruling).
 *
 * Replaces the old dashboard hero card ("STATUS — IDLE / {name} is ready /
 * Ready when you are.", cards/HeroCard.tsx, now deleted) — that tile was
 * oversized and redundant with the rest of Home. This reads the SAME two
 * sources the hero card read, so the concept stays in one place:
 *
 *  - Agent state ('idle' | 'thinking') and reachability — `useDashboard()`
 *    (components/dashboard/useDashboard.ts), which the hero card consumed via
 *    Dashboard.tsx's `data.agent` prop. `error` is the one honest reachability
 *    signal that hook exposes: it flips true the instant a poll to
 *    `/api/dashboard` fails and back to false the instant one succeeds, so it
 *    is used as-is here rather than inventing a second "connectivity" concept.
 *    There is no third, more specific "daemon down" signal for this indicator
 *    to draw on — see useDashboard.ts.
 *
 *  - The persona's display name — `agentName` in lib/store.ts, hydrated from
 *    `GET /api/agent/identity` (`refreshIdentity`, kept live by the
 *    `identity_changed` event via livenessSync.ts). This is config, not a
 *    literal: the store's own fallback ('Agent') is the only default, and it
 *    is never "Henry" — `henry` is a load-bearing id elsewhere, not a display
 *    string.
 */
import type { CSSProperties } from 'react';
import { useCommandCenter } from '../../lib/store';
import type { DashboardData } from '../dashboard/useDashboard';
import { useOrchestratorName } from '../world/shared/useOrchestratorName';
import { space, textSize, font } from '../../styles/tokens';
import { useTheme, type ThemeColors } from '../../styles/useTheme';

export type NavAgentState = 'online' | 'thinking' | 'offline';

const STATE_WORD: Record<NavAgentState, string> = {
  online: 'online',
  thinking: 'thinking',
  offline: 'offline',
};

/**
 * Pure mapping from the dashboard's raw agent state + reachability to the
 * dot's three-way state — unit-testable with no network/store involved.
 * `unreachable` wins over whatever stale `rawState` a prior successful poll
 * left behind: a gray dot is honest, a green or pulsing one built on data the
 * daemon has stopped confirming is not.
 */
export function resolveNavAgentState(rawState: string | undefined, unreachable: boolean): NavAgentState {
  if (unreachable) return 'offline';
  if (rawState === 'thinking') return 'thinking';
  return 'online';
}


function dotColor(colors: ThemeColors, state: NavAgentState): string {
  if (state === 'offline') return colors.stale;
  if (state === 'thinking') return colors.cyan;
  return colors.success;
}

/**
 * The dot alone. `.status-pulse` is an existing, previously-unused keyframe
 * (index.css) — reused here rather than adding a second one. Only applied
 * while thinking AND motion is allowed: reduceMotion drops it to a steady
 * dot, matching every other motion-gated affordance in the rail.
 */
export function NavStatusDot({ state, size = 8 }: { state: NavAgentState; size?: number }) {
  const { colors, reduceMotion } = useTheme();
  const pulse = state === 'thinking' && !reduceMotion;
  return (
    <span
      aria-hidden
      className={pulse ? 'status-pulse' : undefined}
      style={{
        display: 'inline-block', flexShrink: 0,
        width: size, height: size, borderRadius: '50%',
        background: dotColor(colors, state),
      }}
    />
  );
}



/**
 * The agent's status on a view's title line — "Henry · thinking" beside the H1.
 *
 * Jesse, twice: the dot does not belong in the rail. Expanded it was a second
 * line of chrome under Home; collapsed it was a green dot floating next to an
 * icon, marking a row rather than saying anything, and reading as decoration
 * you cannot act on. The agreed home for this is the Home heading, where it
 * sits next to the name of the thing it is the status OF and has room for the
 * word as well as the colour.
 */
export function HeaderAgentStatus({ data, error }: {
  /** The Home view's OWN dashboard payload — passed in, not re-fetched. */
  data: DashboardData | null;
  error: unknown;
}) {
  // Taken as props rather than calling `useDashboard()` here, because that
  // hook is not shared state: every call starts its own 15-second poller. The
  // first version of this component called it, which meant Home fetched
  // /api/dashboard twice on every tick, and the header and the body beneath it
  // could disagree about the agent's state for up to fifteen seconds.
  const { colors } = useTheme();
  const storeName = useCommandCenter(s => s.agentName);
  // ONE resolver for the agent's name, shared with the World.
  //
  // QA found the header reading "Home · Agent online" while the World called
  // the same agent Hank: the header read the store's `agentName`, whose
  // default is the literal word "Agent". The dashboard payload's
  // `agent.name` is no better — it is the default persona ("Henry"), which is
  // why the rail's tests deliberately ignored it. `useOrchestratorName` is the
  // configured name the user chose, is module-cached (so this adds no fetch),
  // and is exactly what the World's nameplate and passport use.
  const configured = useOrchestratorName();
  const name = configured
    ?? (storeName && storeName !== 'Agent' ? storeName : null)
    ?? 'Your agent';
  const state = resolveNavAgentState(data?.agent.state, Boolean(error));
  const word = STATE_WORD[state];
  return (
    <span
      data-testid="header-agent-status"
      role="status"
      aria-label={`${name} is ${word}`}
      style={{
        display: 'inline-flex', alignItems: 'center', gap: space.sm,
        fontFamily: font.body, whiteSpace: 'nowrap',
      }}
    >
      <NavStatusDot state={state} size={7} />
      <span style={{ fontSize: textSize.small, color: colors.textMuted, fontWeight: 500 }}>
        {name}
      </span>
      <span style={{ fontSize: textSize.micro, color: colors.textDim }}>{word}</span>
    </span>
  );
}
