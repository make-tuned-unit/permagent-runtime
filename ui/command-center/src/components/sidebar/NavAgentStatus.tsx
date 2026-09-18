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
import { useDashboard } from '../dashboard/useDashboard';
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

/** Combines the two sources described above into what the nav row needs. */
export function useNavAgentStatus(): { name: string; state: NavAgentState; word: string } {
  const name = useCommandCenter(s => s.agentName);
  const { data, error } = useDashboard();
  const state = resolveNavAgentState(data?.agent.state, error);
  return { name, state, word: STATE_WORD[state] };
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

/** OPEN rail (208px): a quiet line under the Home row — dot, name, state word. */
export function NavStatusLine({ name, state, word }: { name: string; state: NavAgentState; word: string }) {
  const { colors } = useTheme();
  return (
    <div
      data-testid="nav-status-line"
      // Screen readers get one clean sentence; the dot is decorative (aria-hidden).
      aria-label={`${name} is ${word}`}
      style={{
        display: 'flex', alignItems: 'center', gap: space.sm,
        // 34px left inset lines the text up under the row's label (12px row
        // padding + 18px icon + 4px of the row's own icon/label gap).
        padding: `0 ${space.xl}px ${space.md}px 34px`,
        fontFamily: font.body,
      }}
    >
      <NavStatusDot state={state} />
      <span style={{ fontSize: textSize.small, color: colors.text, fontWeight: 500, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
        {name}
      </span>
      <span style={{ fontSize: textSize.micro, color: colors.textDim, whiteSpace: 'nowrap' }}>
        {word}
      </span>
    </div>
  );
}

/**
 * COLLAPSED rail (64px): the dot alone, badged onto the Home row's icon so it
 * reads as belonging to Home rather than floating loose in the rail. Hover
 * uses the sidebar's existing tooltip machinery — `onHover`/`onLeave` are the
 * same `showTooltip`/`hideTooltip` pair every other row already wires through
 * `useSidebarTooltip()`, just given "{name} · {word}" instead of a row label.
 */
export function NavStatusBadge({
  name, state, word, onHover, onLeave,
}: {
  name: string; state: NavAgentState; word: string;
  onHover: (el: HTMLElement | null, label: string) => void;
  onLeave: () => void;
}) {
  return (
    <span
      data-testid="nav-status-badge"
      role="status"
      tabIndex={0}
      aria-label={`${name} is ${word}`}
      onMouseEnter={e => onHover(e.currentTarget, `${name} · ${word}`)}
      onMouseLeave={onLeave}
      onFocus={e => onHover(e.currentTarget, `${name} · ${word}`)}
      onBlur={onLeave}
      style={{
        position: 'absolute', bottom: space.xxs, right: space.xxs,
        lineHeight: 0, borderRadius: '50%',
        cursor: 'default',
      } as CSSProperties}
    >
      <NavStatusDot state={state} size={7} />
    </span>
  );
}
