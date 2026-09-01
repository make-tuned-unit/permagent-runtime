/**
 * Settings → Features — the switches for the off-by-default workers:
 * Initiative, the Decision Playbook, the Concierge, the Steward's git-health
 * sweep, the Guard's security sweep, and The Council. Each is one boolean
 * config key; the daemon loop behind it always runs and re-reads the flag every
 * tick, so a flip here lands at the next tick with no restart.
 *
 * The same key is written by the agent's own page under Settings → Agents (and,
 * for the Guard, by the Models pane) through this same `/config/upsert` call.
 * There is no agent-scoped write path for a flag, so the surfaces cannot drift.
 *
 * Read → optimistic write → revert-on-error, exactly like the Guard toggle.
 * A control is never disabled without saying why; the Concierge toggle stays
 * live even without a Gmail token (the loop is inert until one exists) and the
 * row states the precondition plainly.
 */

import { useEffect, useState } from 'react';
import { H1, Row, Section, Toggle } from '../atoms';
import { api, type CouncilMembers, type CouncilSeat } from '../../../lib/api';
import { useCommandCenter } from '../../../lib/store';
import { font } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import {
  conciergePreconditionCopy,
  FEATURE_ROWS,
  gmailTokenPresent,
  readFlag,
  type FeatureKey,
  type IntegrationStatus,
} from './features';

type PanelProps = { goto: (key: string) => void };

type FlagState = Record<FeatureKey, boolean | null>;

/**
 * Derived, not listed: a hand-written map silently omitted a newly added row,
 * and an omitted key reads as `undefined` — which renders a toggle claiming OFF
 * before the daemon has been asked.
 */
const UNLOADED = Object.fromEntries(FEATURE_ROWS.map(r => [r.key, null])) as FlagState;

export function FeaturesPanel({ goto }: PanelProps) {
  const { colors } = useTheme();
  const [flags, setFlags] = useState<FlagState>(UNLOADED);
  const [errors, setErrors] = useState<Partial<Record<FeatureKey, string>>>({});
  const [integrations, setIntegrations] = useState<IntegrationStatus[] | null>(null);
  const [members, setMembers] = useState<CouncilMembers | null>(null);
  const [membersError, setMembersError] = useState<string | null>(null);

  const configRev = useCommandCenter(s => s.configRev);

  useEffect(() => {
    let active = true;
    for (const row of FEATURE_ROWS) {
      api.readConfig(row.key)
        .then(raw => { if (active) setFlags(f => ({ ...f, [row.key]: readFlag(raw) })); })
        .catch(() => { if (active) setFlags(f => ({ ...f, [row.key]: false })); });
    }
    api.getIntegrations()
      .then(list => { if (active) setIntegrations(list); })
      .catch(() => { if (active) setIntegrations([]); });
    return () => { active = false; };
    // `configRev` = the daemon's `config_changed` frame. These toggles write
    // config keys, and so does the agent; without this the panel showed the
    // flags as they were when it mounted.
  }, [configRev]);

  useEffect(() => {
    if (flags.council_enabled !== true) {
      setMembers(null);
      setMembersError(null);
      return;
    }
    let active = true;
    api.getCouncilMembers()
      .then(m => { if (active) setMembers(m); })
      .catch(err => {
        if (active) {
          setMembersError(err instanceof Error ? err.message : String(err));
        }
      });
    return () => { active = false; };
  }, [flags.council_enabled]);

  const save = (key: FeatureKey, v: boolean) => {
    const prev = flags[key];
    setFlags(f => ({ ...f, [key]: v }));
    setErrors(e => ({ ...e, [key]: undefined }));
    api.upsertConfig(key, v).catch(err => {
      setFlags(f => ({ ...f, [key]: prev }));
      setErrors(e => ({
        ...e,
        [key]: `Couldn't save: ${err instanceof Error ? err.message : String(err)}`,
      }));
    });
  };

  const gmailToken = gmailTokenPresent(integrations);

  return (
    <div>
      <H1 sub="Workers that are off until you switch them on. Each flip is written to config and picked up by the running daemon at its next tick — no restart. Every worker listed here also appears under Settings → Agents whether or not it is switched on, each carrying the same switch — one config key, not two.">
        Features
      </H1>

      <Section
        title="Switches"
        sub="Every one of these only ever proposes; nothing here acts on your behalf without a Decision-Inbox approval."
      >
        {FEATURE_ROWS.map(row => {
          const value = flags[row.key];
          const error = errors[row.key];
          const isConcierge = row.key === 'concierge_enabled';
          return (
            <Row key={row.key} label={row.label} hint={row.what}>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 14 }}>
                {value === null ? (
                  <span style={{ fontSize: 12, color: colors.textDim, paddingTop: 4 }}>Loading…</span>
                ) : (
                  <Toggle on={value} onChange={v => save(row.key, v)} />
                )}
                <div style={{ flex: 1, fontSize: 11, color: colors.textMuted, lineHeight: 1.5, paddingTop: 3 }}>
                  <div>{row.effect}</div>
                  {isConcierge && (
                    <div
                      data-testid="concierge-precondition"
                      style={{ marginTop: 4, color: gmailToken === false ? colors.text : colors.textMuted, fontFamily: font.body }}
                    >
                      {conciergePreconditionCopy(gmailToken)}
                    </div>
                  )}
                  {error && (
                    <div style={{ marginTop: 4, color: colors.danger }}>{error}</div>
                  )}
                </div>
              </div>
            </Row>
          );
        })}
      </Section>

      {flags.council_enabled === true && (
        <CouncilSeats
          members={members}
          error={membersError}
          onSave={(next, prev) => {
            setMembers(next);
            setMembersError(null);
            api.putCouncilMembers(next.exclude)
              .then(saved => setMembers(saved))
              .catch(err => {
                setMembers(prev);
                setMembersError(`Couldn't save seats: ${err instanceof Error ? err.message : String(err)}`);
              });
          }}
        />
      )}

      <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>
        Each worker is listed under{' '}
        <button
          onClick={() => goto('agents')}
          style={{
            background: 'transparent', border: 'none', padding: 0, cursor: 'pointer',
            color: colors.cyan, fontFamily: font.body, fontSize: 12, textDecoration: 'underline',
          }}
        >
          Settings → Agents
        </button>
        {' '}whether or not it is switched on — with the same switch on its own page, and
        its live state once it is on.
      </div>
    </div>
  );
}

function seatedSeats(members: CouncilMembers): CouncilSeat[] {
  return members.seats.filter(s => s.configured && !s.cli_or_acp);
}

function CouncilSeats({
  members,
  error,
  onSave,
}: {
  members: CouncilMembers | null;
  error: string | null;
  onSave: (next: CouncilMembers, prev: CouncilMembers) => void;
}) {
  const { colors } = useTheme();
  const seats = members ? seatedSeats(members) : [];

  const toggle = (provider: string, seated: boolean) => {
    if (!members) return;
    const exclude = seated
      ? members.exclude.filter(p => p.toLowerCase() !== provider.toLowerCase())
      : [...members.exclude.filter(p => p.toLowerCase() !== provider.toLowerCase()), provider];
    const next: CouncilMembers = {
      ...members,
      exclude,
      seats: members.seats.map(s =>
        s.provider === provider ? { ...s, excluded: !seated } : s,
      ),
    };
    onSave(next, members);
  };

  return (
    <Section
      title="Council seats"
      sub="Every connected chat-completion provider sits on the Council unless you drop it here. Coding CLIs (Claude Code, Cursor, Codex) are workers, not debate seats. Unchecking a toy local model keeps it from spending a seat next to Claude."
    >
      {!members && !error && (
        <div style={{ fontSize: 12, color: colors.textDim }}>Loading seats…</div>
      )}
      {error && (
        <div style={{ fontSize: 12, color: colors.danger }}>{error}</div>
      )}
      {members && seats.length === 0 && (
        <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>
          No connected chat providers. Connect a key under Settings → Models, then they will appear here.
        </div>
      )}
      {seats.map(seat => {
        const on = !seat.excluded;
        return (
          <label
            key={seat.provider}
            data-testid={`council-seat-${seat.provider}`}
            style={{
              display: 'flex', alignItems: 'center', gap: 10,
              padding: '8px 0', borderTop: `1px solid ${colors.border}`,
              fontSize: 13, color: colors.text, cursor: 'pointer',
            }}
          >
            <input
              type="checkbox"
              checked={on}
              onChange={e => toggle(seat.provider, e.target.checked)}
            />
            <span style={{ flex: 1 }}>
              {seat.display_name}
              <span style={{ color: colors.textDim, marginLeft: 8, fontFamily: font.body, fontSize: 11 }}>
                {seat.model}
              </span>
            </span>
          </label>
        );
      })}
    </Section>
  );
}
