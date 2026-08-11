import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { HudShell, Section } from './HudShell';

// The Steward — git repo hygiene (crate::steward + the scheduled steward.yaml
// recipe). Read/propose work (commit messages, stale-branch reports, repo
// health) runs autonomously; destructive git ops pass through a safety core
// that lives in CODE, not a prompt: protected branches are hard-refused, and
// anything destructive that clears the guard is routed to the board as an
// approval card.
//
// Like the Reader and Watcher, it has NO live status endpoint (documented
// gap), so this panel invents no metrics — it states honestly what the
// Steward does and where its real output lands (steward-report cards).

const STEWARD_TRIM = AGENT_TRIM.steward; // verdigris patina

interface StewardHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function StewardHUD({ visible, onClose }: StewardHUDProps) {
  if (!visible) return null;

  const statusPill = (
    <div style={{
      display: 'inline-block',
      padding: '2px 8px',
      borderRadius: 3,
      fontSize: 10,
      fontWeight: 700,
      letterSpacing: '0.08em',
      background: 'rgba(127, 168, 144, 0.10)',
      color: STEWARD_TRIM,
      border: `1px solid ${STEWARD_TRIM}44`,
    }}>
      TENDING THE REPOS
    </div>
  );

  return (
    <HudShell visible={visible} onClose={onClose} title="THE STEWARD" statusPill={statusPill}>
      <div style={{ padding: '4px 14px 8px' }}>
        <span style={{ fontSize: 11, color: '#9CA3AF', lineHeight: 1.5 }}>
          The groundskeeper of your repositories — every weekday morning it
          sweeps every repo under your dev root using your own git and GitHub
          login, then files one fleet-health report to the board and the Brain.
          Proposes; does not destroy.
        </span>
      </div>

      <Section title="TENDS" trimColor={STEWARD_TRIM}>
        <Bullet>Fleet sweep: branch drift, uncommitted work, stale branches in every repo</Bullet>
        <Bullet>Orphaned worktrees and red CI, flagged for attention</Bullet>
        <Bullet>Commit messages, changelogs, PR descriptions for the primary repo</Bullet>
        <Bullet>CI-failure investigation</Bullet>
      </Section>

      <Section title="THE SAFETY CORE" trimColor={COLORS.neonAmber}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <span style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.5 }}>
            Destructive git ops (branch deletes, history rewrites, force
            pushes) pass a safety core written in code, not prompt — protected
            branches are refused outright, and anything else destructive
            arrives on the board as a proposal card for your approval.
          </span>
        </div>
      </Section>
    </HudShell>
  );
}

function Bullet({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.7, display: 'flex', gap: 8 }}>
      <span style={{ color: STEWARD_TRIM }}>·</span>
      <span>{children}</span>
    </div>
  );
}
