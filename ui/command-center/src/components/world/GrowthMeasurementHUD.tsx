import { useEffect, useState } from 'react';
import { COLORS } from './constants';
import { AGENT_TRIM, STATE } from './shared/palette';
import { useAgentRuntimeStates } from './shared/agentStatus';
import { HudShell, Section, StatRow } from './HudShell';
import { Chip } from '../common/Chip';
import { apiFetch } from '../../lib/api';
import { space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { GrowthResultsData } from '../grow/growthResults';

// Growth measurement — the pass that closes the Grow loop (crate::growth::sweep
// + growth_sweep.rs). Verify a shipped action, freeze its before-window, then
// as each 7/14/28-day window closes compare after with before against the
// project's own week-to-week swing and write a verdict: helped, hindered, no
// effect, inconclusive, or confounded. It runs every 6 hours and had no seat
// here at all until now (D18's render target) — `growth_sweep.rs` announces
// `agent_state_changed` on the `growth_measurement` id, `working` only while a
// pass genuinely runs, so this pill is a live wire like the Steward's, not the
// honest-static placeholder Polybot's still is. Everything below the pill is
// the same fleet record the Home dashboard's Growth card reads
// (`/api/growth-results`): the running tallies and when the last verdict
// actually landed.

const GROWTH_TRIM = AGENT_TRIM.growthMeasurement;

interface GrowthMeasurementHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function GrowthMeasurementHUD({ visible, onClose }: GrowthMeasurementHUDProps) {
  const { colors } = useTheme();
  const runtime = useAgentRuntimeStates();
  const [data, setData] = useState<GrowthResultsData | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    let active = true;
    apiFetch<GrowthResultsData>('/api/growth-results')
      .then((d) => {
        if (active) {
          setData(d);
          setError(null);
        }
      })
      .catch((e) => {
        if (active) setError(e instanceof Error ? e.message : 'the daemon did not answer');
      });
    return () => {
      active = false;
    };
  }, [visible]);

  if (!visible) return null;

  const live = runtime.find((a) => a.id === 'growth_measurement');
  const isDaemon = live?.source === 'daemon';
  const label =
    isDaemon && live?.hudState === 'working'
      ? 'MEASURING'
      : isDaemon && live?.hudState === 'error'
        ? 'PASS FAILED'
        : isDaemon
          ? 'ON WATCH'
          : 'STANDING BY';
  const pillColor = isDaemon && live?.hudState === 'error' ? STATE.error : GROWTH_TRIM;
  const statusPill = isDaemon ? (
    <Chip kind="state" color={pillColor} pulse={live?.hudState === 'working'}>
      {label}
    </Chip>
  ) : (
    <Chip kind="static" color={pillColor}>
      {label}
    </Chip>
  );

  const fleet = data?.fleet;
  const measured = fleet ? fleet.helped + fleet.hindered + fleet.noEffect + fleet.inconclusive : 0;
  const lastJudgedAt = fleet?.recent?.[0]?.judgedAt ?? null;

  return (
    <HudShell visible={visible} onClose={onClose} title="THE GROWER" statusPill={statusPill}>
      <div style={{ padding: `${space.xs}px 14px ${space.md}px` }}>
        <span style={{ fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.5 }}>
          Every 6 hours: verify a shipped growth action, freeze its
          before-window, then as each 7/14/28-day window closes compare after
          with before against the project's own week-to-week swing.
          Inconclusive is a first-class verdict here, not a failure to decide.
        </span>
      </div>

      <Section title="TALLY" trimColor={GROWTH_TRIM}>
        {error ? (
          <div style={{ fontSize: textSize.micro, color: COLORS.neonAmber, lineHeight: 1.5 }}>
            Couldn't read the growth record — {error}
          </div>
        ) : fleet ? (
          <>
            <StatRow label="Helped" value={fleet.helped} />
            <StatRow label="Hindered" value={fleet.hindered} />
            <StatRow label="No effect" value={fleet.noEffect} />
            <StatRow label="Inconclusive" value={fleet.inconclusive} />
            <StatRow label="Projects" value={fleet.projects} />
          </>
        ) : (
          <div style={{ fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.5 }}>
            Reading…
          </div>
        )}
      </Section>

      <Section title="LAST SWEEP" trimColor={GROWTH_TRIM}>
        <div style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.5 }}>
          {lastJudgedAt
            ? `Last verdict landed ${new Date(lastJudgedAt).toLocaleString()}.`
            : measured === 0
              ? 'Nothing has been judged yet — verify a shipped action to start a window.'
              : 'Reading…'}
        </div>
      </Section>
    </HudShell>
  );
}
