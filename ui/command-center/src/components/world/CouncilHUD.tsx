import { useEffect, useState } from 'react';
import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { HudShell, Section, StatRow } from './HudShell';
import { Chip } from '../common/Chip';
import { councilStatus } from './deskStatus';
import { api, type CouncilLatest } from '../../lib/api';
import { textSize } from '../../styles/tokens';

// The Council of LLMs — every configured provider argues the same brief and a
// chair writes the report (crate::council + council_sweep.rs).
//
// It had no seat in the World at all until J11, while the Guard — off by
// default under the same Features gating — has had one for months. That
// asymmetry was agent-QA D-N5-1.
//
// Its live state is still unreported: there is no council event constructor
// anywhere in the daemon, so nothing can say "a session is convening right
// now". The pill therefore states the CADENCE, which is a fact about the
// calendar (Sunday 22:00 local, Monday catch-up — council/due.rs) and is drawn
// static so it cannot be read as a live status. Everything below the pill is
// real: `/api/council/latest` is the same record the Home card renders.

const COUNCIL_TRIM = AGENT_TRIM.council;

interface CouncilHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function CouncilHUD({ visible, onClose }: CouncilHUDProps) {
  // null = unknown (still reading). Never claim OFF on a failed read.
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [latest, setLatest] = useState<CouncilLatest | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    let active = true;
    api.readConfig('council_enabled')
      .then(r => { if (active) setEnabled(r === true); })
      .catch(() => { /* unknown stays unknown */ });
    api.getCouncilLatest()
      .then(d => { if (active) { setLatest(d); setError(null); } })
      .catch(e => {
        // A daemon that cannot be reached is not a Council with nothing to
        // say, and must not render as one.
        if (active) setError(e instanceof Error ? e.message : 'the daemon did not answer');
      });
    return () => { active = false; };
  }, [visible]);

  if (!visible) return null;

  const status = councilStatus(enabled);
  const session = latest?.session ?? null;
  const report = latest?.report ?? null;

  return (
    <HudShell
      visible={visible}
      onClose={onClose}
      title="THE COUNCIL"
      statusPill={<Chip kind="static" color={COUNCIL_TRIM}>{status.label}</Chip>}
    >
      <div style={{ padding: '4px 14px 8px' }}>
        <span style={{ fontSize: textSize.micro, color: '#9CA3AF', lineHeight: 1.5 }}>
          {enabled === false
            ? 'Off — enable the Council in Settings. When on, every configured provider argues the same brief and a chair writes up where they agreed and where they did not.'
            : 'Every configured provider argues the same brief; a chair writes up the consensus, the dissent, and what to do about it.'}
        </span>
      </div>

      <Section title="WHEN" trimColor={COUNCIL_TRIM}>
        <div style={{ fontSize: textSize.micro, color: '#D1D5DB', lineHeight: 1.5 }}>
          Sundays at 22:00, your local time — with a Monday catch-up if the
          machine was asleep. Nothing reports a session starting yet, so this
          panel states the schedule rather than watching for it.
        </div>
      </Section>

      <Section title="LAST SESSION" trimColor={COUNCIL_TRIM}>
        {error ? (
          <div style={{ fontSize: textSize.micro, color: COLORS.neonAmber, lineHeight: 1.5 }}>
            Couldn't read the Council record — {error}
          </div>
        ) : session ? (
          <>
            <StatRow label="Started" value={new Date(session.started_at).toLocaleString()} />
            <StatRow label="Status" value={session.status} />
            <StatRow label="Open actions" value={latest?.openActions ?? 0} />
            {report && (
              <div style={{ fontSize: textSize.micro, color: '#D1D5DB', lineHeight: 1.5, marginTop: 6 }}>
                {report.headline}
              </div>
            )}
          </>
        ) : (
          <div style={{ fontSize: textSize.micro, color: '#9CA3AF', lineHeight: 1.5 }}>
            {latest === null ? 'Reading…' : 'No session on record yet.'}
          </div>
        )}
      </Section>
    </HudShell>
  );
}
