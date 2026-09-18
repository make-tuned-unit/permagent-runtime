import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { HudShell, Section } from './HudShell';
import { useNudge } from './agents/watcherNudge';
import { Chip } from '../common/Chip';
import { space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

// The Watcher (Echo, #672) — the daemon's proactive worker. It watches the
// Brain and project news and surfaces at most one nudge a day.
//
// Like the Reader, it has NO live status endpoint (documented gap), so this
// panel does not invent metrics — no "memories scanned", no uptime, no queue
// depth. What it CAN show honestly is the one real signal the Watcher emits:
// the `proactive_nudge` event, which watcherNudge.ts already tracks for the
// in-world beacon flare. When a nudge has arrived this session the HUD shows
// its REAL subject and message; otherwise it says plainly that none has.

const WATCHER_TRIM = AGENT_TRIM.watcher; // pale vigil steel-blue

interface WatcherHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function WatcherHUD({ visible, onClose }: WatcherHUDProps) {
  const { colors } = useTheme();
  const nudge = useNudge();

  if (!visible) return null;

  // seq 0 means no nudge has been seen this session (replayed buffer events
  // are deliberately ignored upstream — the world only reflects what it saw).
  const hasNudge = nudge.seq > 0;

  // The split this HUD's own header already describes, now visible in the
  // pill. A nudge is a real event off /events and carries the moment it
  // arrived, so it reads as live and can say when. "KEEPING WATCH" is not
  // backed by any status endpoint — it is what the Watcher is for, not a
  // reading of what it is doing — so it takes the static form and stops
  // borrowing a live pill's clothes.
  const statusPill = hasNudge
    ? <Chip kind="state" color={WATCHER_TRIM} asOf={nudge.at}>NUDGED</Chip>
    : <Chip kind="static" color={WATCHER_TRIM}>KEEPING WATCH</Chip>;

  return (
    <HudShell visible={visible} onClose={onClose} title="THE WATCHER" statusPill={statusPill}>
      <div style={{ padding: `${space.xs}px 14px ${space.md}px` }}>
        <span style={{ fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.5 }}>
          The proactive worker — dormant Brain threads, project news, and (with
          the Financier) overbought sell signals on stocks you already hold.
          News is at most one a day. Holding alerts are daily per symbol.
        </span>
      </div>

      <Section title="WATCHES" trimColor={WATCHER_TRIM}>
        <Bullet>Dormant Brain threads worth returning to</Bullet>
        <Bullet>Fresh news on projects you're building</Bullet>
        <Bullet>Overbought open holdings — Financier scores, Watcher delivers</Bullet>
        <Bullet>News/dormant: at most one a day. Holdings: once per symbol per day</Bullet>
      </Section>

      <Section title="LAST NUDGE" trimColor={COLORS.neonAmber}>
        {hasNudge ? (
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.xs }}>
            <span style={{ fontSize: textSize.micro, color: colors.text, fontWeight: 600, lineHeight: 1.4 }}>
              {nudge.subject || nudge.kind || 'Nudge'}
            </span>
            {nudge.message && (
              <span style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.5 }}>
                {nudge.message}
              </span>
            )}
            <span style={{ fontSize: textSize.micro, color: colors.textMuted }}>
              {new Date(nudge.at).toLocaleString(undefined, {
                month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit',
              })}
              {nudge.count > 1 ? ` · ${nudge.count} this session` : ''}
            </span>
          </div>
        ) : (
          <span style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.5 }}>
            No nudge yet this session. The vigil beacon on the Watcher's tower
            flares when one arrives.
          </span>
        )}
      </Section>
    </HudShell>
  );
}

function Bullet({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.7, display: 'flex', gap: space.md }}>
      <span style={{ color: WATCHER_TRIM }}>·</span>
      <span>{children}</span>
    </div>
  );
}
