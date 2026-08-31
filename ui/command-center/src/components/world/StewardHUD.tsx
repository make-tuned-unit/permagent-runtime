import { useEffect, useState } from 'react';
import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { useAgentRuntimeStates } from './shared/agentStatus';
import { HudShell, Section } from './HudShell';
import { Chip } from '../common/Chip';
import { api } from '../../lib/api';

// The Steward — git repo hygiene (crate::steward + the scheduled steward.yaml
// recipe). Read/propose work (commit messages, stale-branch reports, repo
// health) runs autonomously; destructive git ops pass through a safety core
// that lives in CODE, not a prompt: protected branches are hard-refused, and
// anything destructive that clears the guard is routed to the board as an
// approval card.
//
// Two live paths, kept distinct so the HUD does not over-claim:
//   1. The weekday `git-steward` recipe — a scheduled LLM pass over ~/dev.
//   2. The native sweep (`steward_scan_enabled`, default OFF) — one project
//      per interval, proposals only.
// Daemon `agent_state_changed` events use the worker id `git_steward`; the
// world character is `steward`. The live wire maps the two.

const STEWARD_TRIM = AGENT_TRIM.steward; // verdigris patina

interface StewardHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function StewardHUD({ visible, onClose }: StewardHUDProps) {
  const runtime = useAgentRuntimeStates();
  const [enabled, setEnabled] = useState<boolean | null>(null);
  useEffect(() => {
    if (!visible) return;
    let active = true;
    api.readConfig('steward_scan_enabled')
      .then(r => { if (active) setEnabled(r === true); })
      .catch(() => { /* unknown stays unknown — never claim OFF on a failed read */ });
    return () => { active = false; };
  }, [visible]);

  if (!visible) return null;

  const live = runtime.find(a => a.id === 'steward');
  const isDaemon = live?.source === 'daemon';
  // Live work (recipe or native sweep) wins over the feature-flag pill so a
  // weekday pass is not labelled "off" while it is actually running.
  const label =
    isDaemon && live?.hudState === 'working'
      ? 'SWEEPING'
      : isDaemon && live?.hudState === 'error'
        ? 'SWEEP BLOCKED'
        : enabled === false
          ? 'RECIPE ONLY'
          : isDaemon
            ? 'ON WATCH'
            : 'STANDING BY';
  const pillColor = isDaemon && live?.hudState === 'error' ? '#FF5D5D' : STEWARD_TRIM;

  // A daemon-backed reading is a live one and is drawn as such — filled, with
  // a liveness dot, pulsing only while work is genuinely in flight. Without a
  // daemon behind it the label is a standing fact, not a status, so it takes
  // the outline form that says so.
  const statusPill = isDaemon
    ? <Chip kind="state" color={pillColor} pulse={live?.hudState === 'working'}>{label}</Chip>
    : <Chip kind="static" color={pillColor}>{label}</Chip>;

  return (
    <HudShell visible={visible} onClose={onClose} title="THE STEWARD" statusPill={statusPill}>
      <div style={{ padding: '4px 14px 8px' }}>
        <span style={{ fontSize: 11, color: '#9CA3AF', lineHeight: 1.5 }}>
          {enabled === false
            ? 'The native git-health sweep is off (Settings → Features). The weekday recipe can still file a fleet report. Either way: proposes; does not destroy, commit, or merge.'
            : 'The groundskeeper of your repositories — a weekday fleet pass under your dev root, plus an optional native sweep of one project at a time. Proposes; does not destroy, commit, or merge.'}
        </span>
      </div>

      <Section title="TENDS" trimColor={STEWARD_TRIM}>
        <Bullet>Fleet sweep: branch drift, uncommitted work, stale and gone-upstream branches</Bullet>
        <Bullet>Orphaned worktrees, flagged as proposals when they are merged and clean</Bullet>
        <Bullet>Commit messages, changelogs, PR descriptions for the primary repo</Bullet>
        <Bullet>Lists recent GitHub Actions runs if `gh` is available — does not run CI, and does not dispatch a fix</Bullet>
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
