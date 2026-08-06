import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { useAgentRuntimeStates } from './shared/agentStatus';
import { HudShell, Section } from './HudShell';

// Strix — the security agent (crate::strix + the daemon sweep loop). It probes
// the user's OWN projects for flaws and keeps a living fix checklist.
//
// Unlike the Reader/Watcher/Steward panels, this one CAN show live state: the
// sweep emits agent_state_changed, so the pill below reports what the daemon
// actually said rather than a decorative constant. When no daemon state has
// arrived, it says so plainly instead of inventing a status — the same honesty
// law that governs the sim clamp in agentStatus.ts.

const STRIX_TRIM = AGENT_TRIM.strix; // dark oxblood — the owl at night

interface StrixHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function StrixHUD({ visible, onClose }: StrixHUDProps) {
  const runtime = useAgentRuntimeStates();
  if (!visible) return null;

  const live = runtime.find(a => a.id === 'strix');
  const isDaemon = live?.source === 'daemon';
  const label = !isDaemon
    ? 'STANDING BY'
    : live?.hudState === 'working'
      ? 'SWEEPING'
      : live?.hudState === 'error'
        ? 'SCAN BLOCKED'
        : 'ON WATCH';
  const pillColor = isDaemon && live?.hudState === 'error' ? '#FF5D5D' : STRIX_TRIM;

  const statusPill = (
    <div style={{
      display: 'inline-block',
      padding: '2px 8px',
      borderRadius: 3,
      fontSize: 10,
      fontWeight: 700,
      letterSpacing: '0.08em',
      background: 'rgba(140, 74, 92, 0.12)',
      color: pillColor,
      border: `1px solid ${pillColor}44`,
    }}>
      {label}
    </div>
  );

  return (
    <HudShell visible={visible} onClose={onClose} title="STRIX" statusPill={statusPill}>
      <div style={{ padding: '4px 14px 8px' }}>
        <span style={{ fontSize: 11, color: '#9CA3AF', lineHeight: 1.5 }}>
          Your own projects, probed the way an attacker would — on its own
          cadence, so security review happens without anyone remembering to
          ask. It reports; it never fixes.
        </span>
      </div>

      <Section title="HUNTS FOR" trimColor={STRIX_TRIM}>
        <Bullet>Exposed secrets and credentials in source</Bullet>
        <Bullet>Vulnerable dependencies (CVE / advisory matches)</Bullet>
        <Bullet>Injection, SSRF and access-control weaknesses</Bullet>
        <Bullet>Risky configuration and reachable attack surface</Bullet>
      </Section>

      <Section title="WHAT YOU GET" trimColor={STRIX_TRIM}>
        <div style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.5 }}>
          Each finding lands on its project's Overview as a checklist item
          carrying severity, CWE, where it lives, and how to fix it — the
          standing answer to "what's wrong with this project".
        </div>
      </Section>

      <Section title="THE LEASH" trimColor={COLORS.neonAmber}>
        <div style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.5 }}>
          Scope is enforced in code, not prompt: only your own project roots
          are scannable — URLs, foreign paths and directory traversal are
          refused before the scanner runs. Passive inspection is autonomous;
          anything intrusive is proposed for your approval, and nothing is
          ever remediated automatically.
        </div>
      </Section>
    </HudShell>
  );
}

function Bullet({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.7, display: 'flex', gap: 8 }}>
      <span style={{ color: STRIX_TRIM }}>·</span>
      <span>{children}</span>
    </div>
  );
}
