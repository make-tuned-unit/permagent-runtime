import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { HudShell, Section } from './HudShell';
import { useNudge } from './agents/watcherNudge';

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
  const nudge = useNudge();

  if (!visible) return null;

  // seq 0 means no nudge has been seen this session (replayed buffer events
  // are deliberately ignored upstream — the world only reflects what it saw).
  const hasNudge = nudge.seq > 0;

  const statusPill = (
    <div style={{
      display: 'inline-block',
      padding: '2px 8px',
      borderRadius: 3,
      fontSize: 10,
      fontWeight: 700,
      letterSpacing: '0.08em',
      background: hasNudge ? 'rgba(159, 184, 216, 0.20)' : 'rgba(159, 184, 216, 0.10)',
      color: WATCHER_TRIM,
      border: `1px solid ${WATCHER_TRIM}${hasNudge ? '88' : '44'}`,
    }}>
      {hasNudge ? 'NUDGED' : 'KEEPING WATCH'}
    </div>
  );

  return (
    <HudShell visible={visible} onClose={onClose} title="THE WATCHER" statusPill={statusPill}>
      <div style={{ padding: '4px 14px 8px' }}>
        <span style={{ fontSize: 11, color: '#9CA3AF', lineHeight: 1.5 }}>
          The proactive worker — watches dormant Brain threads and fresh project
          news, and surfaces at most one thing a day. Quiet by design.
        </span>
      </div>

      <Section title="WATCHES" trimColor={WATCHER_TRIM}>
        <Bullet>Dormant Brain threads worth returning to</Bullet>
        <Bullet>Fresh news on projects you're building</Bullet>
        <Bullet>At most one nudge per day — never a feed</Bullet>
      </Section>

      <Section title="LAST NUDGE" trimColor={COLORS.neonAmber}>
        {hasNudge ? (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{ fontSize: 11, color: '#F3F4F6', fontWeight: 600, lineHeight: 1.4 }}>
              {nudge.subject || nudge.kind || 'Nudge'}
            </span>
            {nudge.message && (
              <span style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.5 }}>
                {nudge.message}
              </span>
            )}
            <span style={{ fontSize: 10, color: '#9CA3AF' }}>
              {new Date(nudge.at).toLocaleString(undefined, {
                month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit',
              })}
              {nudge.count > 1 ? ` · ${nudge.count} this session` : ''}
            </span>
          </div>
        ) : (
          <span style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.5 }}>
            No nudge yet this session. The vigil beacon on the Watcher's tower
            flares when one arrives.
          </span>
        )}
      </Section>
    </HudShell>
  );
}

function Bullet({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.7, display: 'flex', gap: 8 }}>
      <span style={{ color: WATCHER_TRIM }}>·</span>
      <span>{children}</span>
    </div>
  );
}
