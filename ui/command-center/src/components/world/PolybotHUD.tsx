import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { HudShell, Section, StatRow } from './HudShell';
import { Chip } from '../common/Chip';
import { useFinanceDesk } from './financeDesk';
import { polybotStatus } from './deskStatus';
import { textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

// Polybot — the autonomous trading process the Finance tab drives. A separate
// process on this machine, not a model: "running" is a fact about the box.
//
// It had no seat in the World at all (agent-QA D22) and it still has no
// `agent_state_changed` emitter, so its orb is `wire: 'static'` — it does not
// animate and it never claims work. What this panel shows instead is the real
// board (`/api/finance` → `polybot::status()`), stamped with when it was read,
// so the difference between "switched off", "not installed", "stopped" and
// "the daemon didn't answer" is on screen rather than collapsed into one word.

const POLYBOT_TRIM = AGENT_TRIM.polybot;

interface PolybotHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function PolybotHUD({ visible, onClose }: PolybotHUDProps) {
  const { colors } = useTheme();
  const reading = useFinanceDesk(visible);
  if (!visible) return null;

  const status = polybotStatus(reading);
  const p = reading.board?.polybot;
  const pillColor = status.unreachable ? COLORS.neonAmber : POLYBOT_TRIM;

  return (
    <HudShell
      visible={visible}
      onClose={onClose}
      title="THE TRADER"
      statusPill={
        status.live
          ? <Chip kind="state" color={pillColor} asOf={reading.asOf} pulse={status.pulse}>{status.label}</Chip>
          : <Chip kind="static" color={pillColor}>{status.label}</Chip>
      }
    >
      <div style={{ padding: '4px 14px 8px' }}>
        <span style={{ fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.5 }}>
          A separate trading process, started and paused from the Finance tab.
          This panel reads its real state; nothing here animates, because no
          event reports what it is doing between reads.
        </span>
      </div>

      {status.unreachable && (
        <Section title="COULDN'T READ IT" trimColor={COLORS.neonAmber}>
          <div style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.5 }}>
            {reading.error ?? 'The finance board did not answer.'} That is a
            failure to ask, not a report that Polybot is off.
          </div>
        </Section>
      )}

      {p && (
        <Section title="STATE" trimColor={POLYBOT_TRIM}>
          <StatRow label="Installed" value={p.found ? 'yes' : 'no'} />
          <StatRow label="Process" value={p.running ? 'up' : 'down'} />
          <StatRow label="Paused" value={p.paused ? 'yes' : 'no'} />
          {p.credentialsReady !== undefined && (
            <StatRow label="Credentials" value={p.credentialsReady ? 'ready' : 'missing'} />
          )}
          {p.tradeCount != null && <StatRow label="Trades" value={p.tradeCount} />}
          {p.stale && (
            <div style={{ fontSize: textSize.micro, color: COLORS.neonAmber, lineHeight: 1.5, marginTop: 6 }}>
              Its numbers are {p.staleDays ?? '—'} days old — shown as history, not as today.
            </div>
          )}
          {p.detail && (
            <div style={{ fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.5, marginTop: 6 }}>
              {p.detail}
            </div>
          )}
        </Section>
      )}

      <Section title="AWAITING" trimColor={COLORS.neonAmber}>
        <div style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.5 }}>
          No `agent_state_changed` is emitted for Polybot anywhere in the
          daemon, so this seat cannot yet show it working. Until one is, the
          figure stands still on purpose.
        </div>
      </Section>
    </HudShell>
  );
}
