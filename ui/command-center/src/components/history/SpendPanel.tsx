/**
 * History → Spend. Per-session and per-project token + dollar consumption with a
 * running total, from the REAL `GET /api/governance/spend` (which aggregates
 * the cost_ledger rollup). Also the full budget the cost-router enforces by
 * gating the agent through the Decision Inbox — BOTH the per-session and the
 * per-task ceilings — set here via `POST /api/governance/budget`. This panel
 * fully supersedes the old Autonomy "Spend cap" sliders.
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { api, apiFetch, type ProjectSpend, type SpendSnapshot } from '../../lib/api';
import { font, radius, space, tabularNums, textSize, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';
import { Card, SectionLabel, StatRow } from '../common/Card';
import { bandColor, bandLabel, formatTokens, formatUsd, timeAgo } from '../settings/format';
import { groupProjectSpend, type NoProjectSpend } from './spendProjects';

/** Width of one dollar-ceiling field. Fits "$ 1000" with room for the stepper. */
export const CAP_FIELD_WIDTH = 168;

/** Reading measure for the Spend column. */
export const SPEND_MAX_WIDTH = 1100;

export function SpendPanel() {
  const { colors } = useTheme();
  const [snap, setSnap] = useState<SpendSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Editable ceiling drafts — session (the "spend cap for this machine") and
  // task (the tighter per-unit-of-work cap the old Autonomy slider wrote).
  const [soft, setSoft] = useState('');
  const [gate, setGate] = useState('');
  const [hard, setHard] = useState('');
  const [taskSoft, setTaskSoft] = useState('');
  const [taskGate, setTaskGate] = useState('');
  const [taskHard, setTaskHard] = useState('');
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
        setTaskSoft(String(s.budget.task.soft));
        setTaskGate(String(s.budget.task.gate));
        setTaskHard(String(s.budget.task.hard));
      })
      .catch(() => setError('Could not load spend.'));
  }, []);

  useEffect(() => { load(); }, [load]);

  // Project roots, so a session run from `/` or `/tmp` isn't shown as a
  // project called "/" or "tmp" (see spendProjects.ts). Null = unknown: the
  // rows are then shown exactly as the server sent them.
  const [projectRoots, setProjectRoots] = useState<string[] | null>(null);
  useEffect(() => {
    let live = true;
    apiFetch<{ rootPath?: string | null }[]>('/api/projects')
      .then(rows => {
        if (!live || !Array.isArray(rows)) return;
        setProjectRoots(rows.map(r => r.rootPath ?? '').filter(Boolean));
      })
      .catch(() => { /* unknown roots: leave the server's rows as they are */ });
    return () => { live = false; };
  }, []);

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
      const ceilings = (s: string, g: string, h: string) => {
        const parsedSoft = parseCeiling(s);
        const parsedGate = parseCeiling(g);
        const parsedHard = parseCeiling(h);
        return {
          ...(parsedSoft !== undefined && { soft: parsedSoft }),
          ...(parsedGate !== undefined && { gate: parsedGate }),
          ...(parsedHard !== undefined && { hard: parsedHard }),
        };
      };
      const patch = {
        session: ceilings(soft, gate, hard),
        task: ceilings(taskSoft, taskGate, taskHard),
      };
      const budget = await api.setBudget(patch);
      setSnap(prev => (prev ? { ...prev, budget } : prev));
      setSoft(String(budget.session.soft));
      setGate(String(budget.session.gate));
      setHard(String(budget.session.hard));
      setTaskSoft(String(budget.task.soft));
      setTaskGate(String(budget.task.gate));
      setTaskHard(String(budget.task.hard));
      setSaved(true);
      return true;
    } catch {
      setError('Could not save the budget.');
      // `false` is the Button contract's "it failed": this catch swallows the
      // error into a message, so without it a rejected save would still tick.
      return false;
    } finally {
      setSaving(false);
    }
  }, [soft, gate, hard, taskSoft, taskGate, taskHard]);

  if (error && snap === null) {
    return <div style={{ color: colors.textMuted, fontSize: textSize.small }}>{error}</div>;
  }
  if (snap === null) {
    return <div style={{ color: colors.textDim, fontSize: textSize.small }}>Loading spend…</div>;
  }

  // A two-digit dollar ceiling, not a paragraph: the field is sized to its
  // content. These were `flex: 1` in a full-width row, so on a wide window each
  // ceiling got a third of the card — QA 2026-09-20, "improperly spaced".
  const numInput = (v: string, on: (s: string) => void, label: string, scope = 'Session') => (
    <label style={{ display: 'flex', flexDirection: 'column', gap: space.xs, flex: '0 0 auto', width: CAP_FIELD_WIDTH }}>
      <span style={{ ...type.label, color: colors.textDim }}>{label}</span>
      <div style={{ display: 'flex', alignItems: 'center', gap: space.xs, height: 30, padding: '0 8px', borderRadius: radius.md, background: colors.inputBg, border: `1px solid ${colors.border}` }}>
        <span style={{ color: colors.textDim, fontSize: textSize.small }}>$</span>
        <input
          type="number" min="0" step="0.5" value={v}
          onChange={e => on(e.target.value)}
          aria-label={`${scope} ${label} ceiling in USD`}
          style={{ flex: 1, minWidth: 0, background: 'transparent', border: 'none', outline: 'none', color: colors.text, fontFamily: font.mono, fontSize: textSize.small, ...tabularNums }}
        />
      </div>
    </label>
  );

  const spendFigure = (costUsd: number, tokens: number) => (
    <span style={{ fontFamily: font.mono, fontSize: textSize.small, color: colors.text, ...tabularNums }}>
      {formatUsd(costUsd)}
      <span style={{ color: colors.textDim, marginLeft: space.md }}>{formatTokens(tokens)}</span>
    </span>
  );

  return (
    // The panel is a column of single-column cards, so it reads better at a
    // measure than stretched across a 1900px window.
    <div style={{ display: 'flex', flexDirection: 'column', gap: space.xxl, maxWidth: SPEND_MAX_WIDTH }}>
      {/* Running total */}
      <Card>
        <SectionLabel>Running total — everything you have spent</SectionLabel>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: space.xxxl, marginTop: space.lg, flexWrap: 'wrap' }}>
          <div style={{ ...type.display, fontFamily: font.display, color: colors.cyan, ...tabularNums }}>
            {formatUsd(snap.runningTotalUsd)}
          </div>
          <div style={{ fontSize: textSize.small, color: colors.textMuted, ...tabularNums }}>
            {formatTokens(snap.totalTokens)} tokens · {snap.sessionCount} session{snap.sessionCount === 1 ? '' : 's'}
          </div>
        </div>
      </Card>

      {/* Budget */}
      <Card>
        <SectionLabel>Budget — optional caps you set</SectionLabel>
        <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginTop: space.md, marginBottom: space.xl }}>
          Enforced locally: at the gate ceiling the agent pauses and asks in the Decision Inbox before spending more; at the hard ceiling it stops and preserves your work. Leave high to run uncapped.
        </div>
        <div style={{ ...type.label, color: colors.textDim, marginBottom: space.sm }}>Per session</div>
        <div style={{ display: 'flex', gap: space.lg, alignItems: 'flex-end', flexWrap: 'wrap' }}>
          {numInput(soft, setSoft, 'Warn (soft)')}
          {numInput(gate, setGate, 'Ask (gate)')}
          {numInput(hard, setHard, 'Stop (hard)')}
        </div>
        <div style={{ ...type.label, color: colors.textDim, margin: `${space.xxl}px 0 ${space.sm}px` }}>Per task</div>
        <div style={{ display: 'flex', gap: space.lg, alignItems: 'flex-end', flexWrap: 'wrap' }}>
          {numInput(taskSoft, setTaskSoft, 'Warn (soft)', 'Task')}
          {numInput(taskGate, setTaskGate, 'Ask (gate)', 'Task')}
          {numInput(taskHard, setTaskHard, 'Stop (hard)', 'Task')}
        </div>
        <div style={{ display: 'flex', gap: space.lg, alignItems: 'center', marginTop: 14 }}>
          <Button
            colors={colors}
            variant="primary"
            onClick={saveBudget}
            disabled={saving}
            style={{
              '--pa-btn-pad': '0 16px',
              '--pa-btn-radius': `${radius.md}px`,
              height: 30,
              fontFamily: font.body,
              fontSize: textSize.caption,
            } as CSSProperties}
          >
            {saving ? 'Saving…' : 'Save caps'}
          </Button>
          {saved && <span style={{ fontSize: textSize.caption, color: colors.success }}>Saved</span>}
        </div>
        {error && <div style={{ color: colors.textMuted, fontSize: textSize.caption, marginTop: space.md }}>{error}</div>}
      </Card>

      {/* Per-project */}
      <Card>
        <SectionLabel>By project</SectionLabel>
        {snap.projects.length === 0 ? (
          <div style={{ color: colors.textMuted, fontSize: textSize.small, marginTop: space.lg }}>No spend recorded yet.</div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.md, marginTop: space.xl }}>
            {projectSpendRows(snap.projects, projectRoots).map(row => row.kind === 'project' ? (
              <StatRow
                key={row.p.path}
                left={row.p.label}
                sub={`${row.p.path} · ${row.p.sessionCount} session${row.p.sessionCount === 1 ? '' : 's'}`}
                right={spendFigure(row.p.costUsd, row.p.tokens)}
              />
            ) : (
              <NoProjectRow key="__no_project__" group={row.group} figure={spendFigure} />
            ))}
          </div>
        )}
      </Card>

      {/* Per-session */}
      <Card>
        <SectionLabel>By session</SectionLabel>
        {snap.sessions.length === 0 ? (
          <div style={{ color: colors.textMuted, fontSize: textSize.small, marginTop: space.lg }}>No spend recorded yet.</div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.md, marginTop: space.xl }}>
            {snap.sessions.map(s => (
              <StatRow
                key={s.id}
                left={s.name || s.id}
                sub={`${s.sessionType}${s.updatedAt ? ` · ${timeAgo(s.updatedAt)}` : ''}`}
                right={
                  <span style={{ display: 'flex', alignItems: 'center', gap: space.lg }}>
                    {s.band !== 'ok' && (
                      <Tooltip content={`This session is ${bandLabel(s.band)}`}>
                        <span
                          tabIndex={0}
                          style={{ ...type.label, fontWeight: 700, color: bandColor(s.band, colors) }}
                        >
                          {s.band}
                        </span>
                      </Tooltip>
                    )}
                    <span style={{ fontFamily: font.mono, fontSize: textSize.small, color: colors.text, ...tabularNums }}>
                      {formatUsd(s.costUsd)}
                      <span style={{ color: colors.textDim, marginLeft: space.md }}>{formatTokens(s.tokens)}</span>
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

type ProjectSpendRow =
  | { kind: 'project'; p: ProjectSpend }
  | { kind: 'none'; group: NoProjectSpend };

/** Project rows plus one folded "No project" row, all in cost order. */
function projectSpendRows(rows: ProjectSpend[], roots: string[] | null): ProjectSpendRow[] {
  const { projects, noProject } = groupProjectSpend(rows, roots);
  const out: ProjectSpendRow[] = projects.map(p => ({ kind: 'project' as const, p }));
  if (noProject) {
    const at = out.findIndex(r => r.kind === 'project' && r.p.costUsd < noProject.costUsd);
    out.splice(at === -1 ? out.length : at, 0, { kind: 'none', group: noProject });
  }
  return out;
}

/** Sessions run outside any project, with the folders they ran in. */
function NoProjectRow({ group, figure }: {
  group: NoProjectSpend;
  figure: (costUsd: number, tokens: number) => React.ReactNode;
}) {
  const { colors } = useTheme();
  const folders = group.dirs.length;
  return (
    <div data-testid="spend-no-project">
      <StatRow
        left="No project"
        sub={`${group.sessionCount} session${group.sessionCount === 1 ? '' : 's'} in ${folders} folder${folders === 1 ? '' : 's'} outside any project`}
        right={figure(group.costUsd, group.tokens)}
      />
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.xxs, margin: `${space.xs}px 0 0 ${space.xxl}px` }}>
        {group.dirs.map(d => (
          <div key={d.path} style={{
            display: 'flex', alignItems: 'baseline', gap: space.lg,
            padding: `${space.xxs}px ${space.xl}px`, fontSize: textSize.micro, color: colors.textMuted,
          }}>
            <span style={{ flex: 1, minWidth: 0, fontFamily: font.mono, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {d.path} · {d.sessionCount} session{d.sessionCount === 1 ? '' : 's'}
            </span>
            <span style={{ fontFamily: font.mono, color: colors.text, flexShrink: 0, ...tabularNums }}>{formatUsd(d.costUsd)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
