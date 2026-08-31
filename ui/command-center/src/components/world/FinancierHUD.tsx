import { useEffect, useState } from 'react';
import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { useAgentRuntimeStates } from './shared/agentStatus';
import { HudShell, Section } from './HudShell';
import { Chip } from '../common/Chip';
import { navigateToTool } from '../../lib/store';

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
  const pillColor = isDaemon && live?.hudState === 'error' ? '#FF5D5D' : FINANCIER_TRIM;

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
        <span style={{ fontSize: 11, color: '#9CA3AF', lineHeight: 1.5 }}>
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
        <div style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.5 }}>
          No size advice. Permagent does not hold CLOB keys. Starting Polybot
          lets that bot trade on Polymarket.
        </div>
      </Section>

      <div style={{ padding: '8px 14px 12px' }}>
        <button
          type="button"
          onClick={() => {
            const ok = navigateToTool('finance');
            setTabHint(!ok);
            if (ok) onClose();
          }}
          style={{
            fontSize: 11,
            fontWeight: 600,
            letterSpacing: '0.04em',
            background: 'transparent',
            color: FINANCIER_TRIM,
            border: `1px solid ${FINANCIER_TRIM}66`,
            borderRadius: 3,
            padding: '6px 10px',
            cursor: 'pointer',
          }}
        >
          OPEN THE FINANCE TAB
        </button>
        {tabHint && (
          <div style={{ fontSize: 11, color: '#9CA3AF', marginTop: 8 }}>
            The Finance tab is not in this workspace yet — it is added on the next daemon start.
          </div>
        )}
      </div>
    </HudShell>
  );
}

function Bullet({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.7, display: 'flex', gap: 8 }}>
      <span style={{ color: FINANCIER_TRIM }}>·</span>
      <span>{children}</span>
    </div>
  );
}
