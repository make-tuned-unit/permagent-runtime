import { useEffect, useState, type CSSProperties } from 'react';
import { COLORS } from './constants';
import { AGENT_TRIM, STATE } from './shared/palette';
import { useAgentRuntimeStates } from './shared/agentStatus';
import { HudShell, Section } from './HudShell';
import { Button } from '../common/Button';
import { Chip } from '../common/Chip';
import { navigateToTool } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';
import { textSize } from '../../styles/tokens';

// The Financier — market research and the Finance tab ledger. Reports numbers;
// never sizes a position and cannot place an order. Live state comes from the
// finance tools announcing working/available on the `financier` id.

const FINANCIER_TRIM = AGENT_TRIM.financier;

interface FinancierHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function FinancierHUD({ visible, onClose }: FinancierHUDProps) {
  const runtime = useAgentRuntimeStates();
  const [tabHint, setTabHint] = useState(false);
  const { colors } = useTheme();

  useEffect(() => {
    if (!visible) setTabHint(false);
  }, [visible]);

  if (!visible) return null;

  const live = runtime.find(a => a.id === 'financier');
  const isDaemon = live?.source === 'daemon';
  const label = !isDaemon
    ? 'STANDING BY'
    : live?.hudState === 'working'
      ? 'RESEARCHING'
      : live?.hudState === 'error'
        ? 'QUOTE FAILED'
        : 'ON THE LEDGER';
  const pillColor = isDaemon && live?.hudState === 'error' ? STATE.error : FINANCIER_TRIM;

  // A daemon-backed reading is a live one and is drawn as such — filled, with
  // a liveness dot, pulsing only while work is genuinely in flight. Without a
  // daemon behind it the label is a standing fact, not a status, so it takes
  // the outline form that says so.
  const statusPill = isDaemon
    ? <Chip kind="state" color={pillColor} pulse={live?.hudState === 'working'}>{label}</Chip>
    : <Chip kind="static" color={pillColor}>{label}</Chip>;

  return (
    <HudShell visible={visible} onClose={onClose} title="THE FINANCIER" statusPill={statusPill}>
      <div style={{ padding: '4px 14px 8px' }}>
        <span style={{ fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.5 }}>
          Owns the Finance tab. The Orchestrator can see the board and
          queries this desk for prices, the ledger, and Polybot. The Watcher
          delivers overbought holding alerts. Keys stay in the keychain;
          it never sizes a trade itself.
        </span>
      </div>

      <Section title="KEEPS" trimColor={FINANCIER_TRIM}>
        <Bullet>Live quotes on the watchlist (fetched at read time, never stored)</Bullet>
        <Bullet>Research notes, optionally tied to a ticker</Bullet>
        <Bullet>A record of positions you say you already took</Bullet>
        <Bullet>Overbought signs on open lots — the Watcher notifies</Bullet>
        <Bullet>Optional: your own stock scanner, if it is running</Bullet>
      </Section>

      <Section title="THE LEASH" trimColor={COLORS.neonAmber}>
        <div style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.5 }}>
          No size advice. Permagent does not hold CLOB keys. Starting Polybot
          lets that bot trade on Polymarket.
        </div>
      </Section>

      <div style={{ padding: '8px 14px 12px' }}>
        <Button
          colors={colors}
          variant="ghostOn"
          type="button"
          onClick={() => {
            const ok = navigateToTool('finance');
            setTabHint(!ok);
            if (ok) onClose();
          }}
          style={{
            '--pa-btn-bg': 'transparent',
            '--pa-btn-fg': FINANCIER_TRIM,
            '--pa-btn-border': `${FINANCIER_TRIM}66`,
            '--pa-btn-bg-hover': `${FINANCIER_TRIM}1F`,
            '--pa-btn-border-hover': FINANCIER_TRIM,
            '--pa-btn-bg-active': 'transparent',
            '--pa-btn-pad': '6px 10px',
            '--pa-btn-radius': '3px',
            '--pa-btn-weight': 600,
            fontSize: textSize.micro,
            letterSpacing: '0.04em',
          } as CSSProperties}
        >
          OPEN THE FINANCE TAB
        </Button>
        {tabHint && (
          <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 8 }}>
            The Finance tab is not in this workspace yet — it is added on the next daemon start.
          </div>
        )}
      </div>
    </HudShell>
  );
}

function Bullet({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.7, display: 'flex', gap: 8 }}>
      <span style={{ color: FINANCIER_TRIM }}>·</span>
      <span>{children}</span>
    </div>
  );
}
