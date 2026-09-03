import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { HudShell, Section, StatRow } from './HudShell';
import { Chip } from '../common/Chip';
import { useFinanceDesk } from './financeDesk';
import { pickerStatus } from './deskStatus';
import { space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

// The Picker — the close-scan desk that ranks tomorrow's candidates
// (`picker_close_scan.rs`, judged at the 15:30 ET close).
//
// D22, precisely: the Picker DOES announce, but under the `financier` id, so
// its work has been lighting the Financier's orb and nothing here. Fixing the
// attribution is a daemon change; until it lands this seat stays
// `wire: 'static'` and does not animate.
//
// What is real and readable today is the scanner itself — reachable, scanning,
// last scan date, how many results — straight off the finance board, stamped
// with when it was read. `scan_in_progress` is the one genuine in-flight fact
// these desks can report, so it is the one thing here allowed to pulse.

const PICKER_TRIM = AGENT_TRIM.picker;

interface PickerHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function PickerHUD({ visible, onClose }: PickerHUDProps) {
  const { colors } = useTheme();
  const reading = useFinanceDesk(visible);
  if (!visible) return null;

  const status = pickerStatus(reading);
  const p = reading.board?.picker;
  const pillColor = status.unreachable ? COLORS.neonAmber : PICKER_TRIM;

  return (
    <HudShell
      visible={visible}
      onClose={onClose}
      title="THE PICKER"
      statusPill={
        status.live
          ? <Chip kind="state" color={pillColor} asOf={reading.asOf} pulse={status.pulse}>{status.label}</Chip>
          : <Chip kind="static" color={pillColor}>{status.label}</Chip>
      }
    >
      <div style={{ padding: `${space.xs}px 14px ${space.md}px` }}>
        <span style={{ fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.5 }}>
          Ranks tomorrow's candidates off the closing scan. It reports
          numbers and never sizes a position.
        </span>
      </div>

      {status.unreachable && (
        <Section title="SCANNER" trimColor={COLORS.neonAmber}>
          <div style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.5 }}>
            {reading.error
              ? `Couldn't read the finance board — ${reading.error}.`
              : `The scanner at ${p?.baseUrl ?? 'its configured address'} is not answering.`}
            {' '}A desk that cannot be reached is not a desk that is switched off.
          </div>
        </Section>
      )}

      {p && (
        <Section title="SCAN" trimColor={PICKER_TRIM}>
          <StatRow label="Scanner" value={p.reachable ? 'answering' : 'not answering'} />
          <StatRow label="In progress" value={p.scanInProgress ? 'yes' : 'no'} />
          <StatRow label="Last scan" value={p.scanDate ?? 'none on record'} />
          {p.results != null && <StatRow label="Results" value={p.results} />}
          {reading.board?.pickerUniverseCount != null && (
            <StatRow label="Universe" value={reading.board.pickerUniverseCount} />
          )}
          {p.detail && (
            <div style={{ fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.5, marginTop: space.sm }}>
              {p.detail}
            </div>
          )}
        </Section>
      )}

      <Section title="AWAITING" trimColor={COLORS.neonAmber}>
        <div style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.5 }}>
          The Picker's own runs announce under the Financier's id, so its work
          currently lights that orb instead of this one. Until the attribution
          is fixed in the daemon, this figure stands still — the scan facts
          above are read, not watched.
        </div>
      </Section>
    </HudShell>
  );
}
