/**
 * Governance → Spend. The one panel that needed a new read: per-session and
 * per-project token + dollar consumption with a running total, from the REAL
 * `GET /api/governance/spend` (which aggregates the cost_ledger rollup). Also
 * the optional budget: the session ceiling the cost-router enforces by gating
 * the agent through the Decision Inbox — set here via `POST /api/governance/budget`.
 */

import { useCallback, useEffect, useState } from 'react';
import { api, type SpendSnapshot } from '../../lib/api';
import { font, tabularNums } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Card, SectionLabel, StatRow } from './atoms';
import { bandColor, bandLabel, formatTokens, formatUsd, timeAgo } from './format';

export function SpendPanel() {
  const { colors } = useTheme();
  const [snap, setSnap] = useState<SpendSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Editable session-ceiling draft (the "spend cap for this machine").
  const [soft, setSoft] = useState('');
  const [gate, setGate] = useState('');
  const [hard, setHard] = useState('');
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  const load = useCallback(() => {
    api.getSpend(50)
      .then(s => {
        setSnap(s);
        setError(null);
        setSoft(String(s.budget.session.soft));
        setGate(String(s.budget.session.gate));
        setHard(String(s.budget.session.hard));
      })
      .catch(() => setError('Could not load spend.'));
  }, []);

  useEffect(() => { load(); }, [load]);

  const saveBudget = useCallback(async () => {
    setSaving(true);
    setSaved(false);
    setError(null);
    try {
      const parseCeiling = (value: string): number | undefined => {
        if (value.trim() === '') return undefined;
        const parsed = Number(value);
        if (!Number.isFinite(parsed) || parsed < 0) {
          throw new Error('invalid budget');
        }
        return parsed;
      };
      const parsedSoft = parseCeiling(soft);
      const parsedGate = parseCeiling(gate);
      const parsedHard = parseCeiling(hard);
      const patch = {
        session: {
          ...(parsedSoft !== undefined && { soft: parsedSoft }),
          ...(parsedGate !== undefined && { gate: parsedGate }),
          ...(parsedHard !== undefined && { hard: parsedHard }),
        },
      };
      const budget = await api.setBudget(patch);
      setSnap(prev => (prev ? { ...prev, budget } : prev));
      setSoft(String(budget.session.soft));
      setGate(String(budget.session.gate));
      setHard(String(budget.session.hard));
      setSaved(true);
    } catch {
      setError('Could not save the budget.');
    } finally {
      setSaving(false);
    }
  }, [soft, gate, hard]);

  if (error && snap === null) {
    return <div style={{ color: colors.textMuted, fontSize: 13 }}>{error}</div>;
  }
  if (snap === null) {
    return <div style={{ color: colors.textDim, fontSize: 13 }}>Loading spend…</div>;
  }

  const numInput = (v: string, on: (s: string) => void, label: string) => (
    <label style={{ display: 'flex', flexDirection: 'column', gap: 4, flex: 1, minWidth: 90 }}>
      <span style={{ fontSize: 10, letterSpacing: '0.06em', textTransform: 'uppercase', color: colors.textDim }}>{label}</span>
      <div style={{ display: 'flex', alignItems: 'center', gap: 4, height: 30, padding: '0 8px', borderRadius: 8, background: colors.inputBg, border: `1px solid ${colors.border}` }}>
        <span style={{ color: colors.textDim, fontSize: 13 }}>$</span>
        <input
          type="number" min="0" step="0.5" value={v}
          onChange={e => on(e.target.value)}
          aria-label={`Session ${label} ceiling in USD`}
          style={{ flex: 1, minWidth: 0, background: 'transparent', border: 'none', outline: 'none', color: colors.text, fontFamily: font.mono, fontSize: 13, ...tabularNums }}
        />
      </div>
    </label>
  );

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {/* Running total */}
      <Card>
        <SectionLabel>Running total — everything you have spent</SectionLabel>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 20, marginTop: 10, flexWrap: 'wrap' }}>
          <div style={{ fontFamily: font.display, fontSize: 36, fontWeight: 600, letterSpacing: '-0.02em', color: colors.cyan, ...tabularNums }}>
            {formatUsd(snap.runningTotalUsd)}
          </div>
          <div style={{ fontSize: 13, color: colors.textMuted, ...tabularNums }}>
            {formatTokens(snap.totalTokens)} tokens · {snap.sessionCount} session{snap.sessionCount === 1 ? '' : 's'}
          </div>
        </div>
      </Card>

      {/* Budget */}
      <Card>
        <SectionLabel>Session budget — an optional cap you set</SectionLabel>
        <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 8, marginBottom: 12 }}>
          Enforced locally: at the gate ceiling the agent pauses and asks in the Decision Inbox before spending more; at the hard ceiling it stops and preserves your work. Leave high to run uncapped.
        </div>
        <div style={{ display: 'flex', gap: 10, alignItems: 'flex-end', flexWrap: 'wrap' }}>
          {numInput(soft, setSoft, 'Warn (soft)')}
          {numInput(gate, setGate, 'Ask (gate)')}
          {numInput(hard, setHard, 'Stop (hard)')}
          <button
            onClick={saveBudget}
            disabled={saving}
            style={{
              height: 30, padding: '0 16px', borderRadius: 8,
              background: colors.cyan, border: 'none', color: colors.textOnCyan,
              cursor: saving ? 'default' : 'pointer', fontFamily: font.body, fontSize: 12, fontWeight: 600,
            }}
          >
            {saving ? 'Saving…' : 'Save cap'}
          </button>
          {saved && <span style={{ fontSize: 12, color: colors.success }}>Saved</span>}
        </div>
        {error && <div style={{ color: colors.textMuted, fontSize: 12, marginTop: 8 }}>{error}</div>}
      </Card>

      {/* Per-project */}
      <Card>
        <SectionLabel>By project</SectionLabel>
        {snap.projects.length === 0 ? (
          <div style={{ color: colors.textMuted, fontSize: 13, marginTop: 10 }}>No spend recorded yet.</div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 12 }}>
            {snap.projects.map(p => (
              <StatRow
                key={p.path}
                left={p.label}
                sub={`${p.path} · ${p.sessionCount} session${p.sessionCount === 1 ? '' : 's'}`}
                right={
                  <span style={{ fontFamily: font.mono, fontSize: 13, color: colors.text, ...tabularNums }}>
                    {formatUsd(p.costUsd)}
                    <span style={{ color: colors.textDim, marginLeft: 8 }}>{formatTokens(p.tokens)}</span>
                  </span>
                }
              />
            ))}
          </div>
        )}
      </Card>

      {/* Per-session */}
      <Card>
        <SectionLabel>By session</SectionLabel>
        {snap.sessions.length === 0 ? (
          <div style={{ color: colors.textMuted, fontSize: 13, marginTop: 10 }}>No spend recorded yet.</div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 12 }}>
            {snap.sessions.map(s => (
              <StatRow
                key={s.id}
                left={s.name || s.id}
                sub={`${s.sessionType}${s.updatedAt ? ` · ${timeAgo(s.updatedAt)}` : ''}`}
                right={
                  <span style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                    {s.band !== 'ok' && (
                      <span
                        title={`This session is ${bandLabel(s.band)}`}
                        style={{ fontSize: 10, fontWeight: 700, letterSpacing: '0.04em', textTransform: 'uppercase', color: bandColor(s.band, colors) }}
                      >
                        {s.band}
                      </span>
                    )}
                    <span style={{ fontFamily: font.mono, fontSize: 13, color: colors.text, ...tabularNums }}>
                      {formatUsd(s.costUsd)}
                      <span style={{ color: colors.textDim, marginLeft: 8 }}>{formatTokens(s.tokens)}</span>
                    </span>
                  </span>
                }
              />
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
