/**
 * Settings → Features — the switches for the off-by-default workers:
 * Initiative, the Decision Playbook, the Concierge, the Steward's git-health
 * sweep and the Guard's security sweep. Each is one boolean config key; the
 * daemon loop behind it always runs and re-reads the flag every tick, so a flip
 * here lands at the next tick with no restart.
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
import { api } from '../../../lib/api';
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
  }, []);

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
