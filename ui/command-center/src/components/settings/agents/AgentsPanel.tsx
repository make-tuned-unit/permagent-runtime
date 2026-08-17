/**
 * Settings → Agents — three honest populations (workers, dispatch roster,
 * capabilities) over the merged /api/agents surface. No HUD rebuild: the World
 * view owns the HUDs, and this page deep-links into it for the agents that have
 * an in-world character (see lib/worldAgentIds — the two id namespaces differ).
 */

import { useCallback, useEffect, useState } from 'react';
import { Chip, H1, Section, TextInput } from '../atoms';
import {
  availabilityLabel,
  defaultEnabledLabel,
  EMPTY_ACTIVITY_NOTE,
  EMPTY_GOALS_NOTE,
  EMPTY_JOBS_NOTE,
  EMPTY_SPEND_NOTE,
  engineLabel,
  grantsNotEnforcedNote,
  grantsSummary,
  liveStateLabel,
  presenceLabel,
  requiredSecretsLabel,
  truncatedNote,
  type LabelTone,
} from '../agentsPanel';
import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useCommandCenter } from '../../../lib/store';
import {
  fetchAgentDetail,
  fetchAgentWork,
  fetchRoster,
  saveGrants,
  saveSecret,
  type AgentDetail,
  type BackgroundWorker,
  type Capability,
  type DispatchPersona,
  type ListSection,
  type RosterResponse,
  type Secrets,
  type WorkReview,
} from '../../../lib/agentsApi';
import { worldAgentIdForAgent } from '../../../lib/worldAgentIds';
import { ROSTER } from '../../world/agents/roster';

type PanelProps = { goto: (key: string) => void };

type GrantMode = 'inherit' | 'nothing' | 'narrowed';

function toneColor(tone: LabelTone, colors: ReturnType<typeof useTheme>['colors']): string {
  if (tone === 'ok') return colors.cyan;
  if (tone === 'error') return colors.danger;
  return colors.textMuted;
}

function StatusText({ text, tone }: { text: string; tone: LabelTone }) {
  const { colors } = useTheme();
  return (
    <span style={{ fontSize: 11, color: toneColor(tone, colors), fontWeight: tone === 'error' ? 600 : 400 }}>
      {text}
    </span>
  );
}

function grantsToMode(grants: DispatchPersona['grants']): GrantMode {
  if (grants.mode === 'inherit_global') return 'inherit';
  if (grants.extensions.length === 0) return 'nothing';
  return 'narrowed';
}

function WorldLink({ agentId }: { agentId: string }) {
  const { colors } = useTheme();
  const [unreachable, setUnreachable] = useState(false);
  const worldId = worldAgentIdForAgent(agentId);
  if (!worldId || !ROSTER.some(a => a.id === worldId)) {
    return (
      <div style={{ fontSize: 12, color: colors.textDim, lineHeight: 1.5 }}>
        This agent has no in-world presence.
      </div>
    );
  }
  return (
    <div>
      <button
        type="button"
        onClick={() => setUnreachable(!useCommandCenter.getState().focusWorldAgent(worldId))}
        style={{
          fontSize: 12, fontWeight: 600, color: colors.cyan, background: 'none',
          border: 'none', cursor: 'pointer', padding: 0, fontFamily: font.body,
        }}
      >
        Open in World
      </button>
      {unreachable && (
        <div style={{ fontSize: 11, color: colors.warning, marginTop: 6, lineHeight: 1.45 }}>
          No workspace has the World view open, so there is nowhere to fly to. Add it to a
          workspace first.
        </div>
      )}
    </div>
  );
}

function SecretsEditor({
  agentId,
  secrets,
  onRefresh,
}: {
  agentId: string;
  secrets: Secrets;
  onRefresh: () => Promise<void>;
}) {
  const { colors } = useTheme();
  const [name, setName] = useState('');
  const [value, setValue] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const write = async (secretName: string, secretValue: string | null) => {
    setBusy(true);
    setError(null);
    try {
      await saveSecret(agentId, secretName, secretValue);
      setName('');
      setValue(''); // clear so a written value never lingers in React state
      await onRefresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not save secret');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <div style={{ fontSize: 12, color: colors.textMuted, marginBottom: 10, lineHeight: 1.5 }}>
        Write-only: presence is shown, values never are. Setting a name again replaces it; Remove deletes.
      </div>
      {secrets.status === 'unavailable' ? (
        <div style={{ fontSize: 12, color: colors.danger }}>
          Secrets could not be read — {secrets.reason}
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginBottom: 14 }}>
          {secrets.items.length === 0 && (
            <div style={{ fontSize: 12, color: colors.textDim }}>No per-agent secrets listed.</div>
          )}
          {secrets.items.map(item => {
            const label = presenceLabel(item.presence);
            return (
              <div
                key={item.name}
                data-testid={`secret-row-${item.name}`}
                style={{
                  display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12,
                  fontSize: 12, fontFamily: font.body,
                }}
              >
                <span style={{ color: colors.text, fontFamily: font.mono }}>{item.name}</span>
                <span style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <StatusText text={label.text} tone={label.tone} />
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => { void write(item.name, null); }}
                    style={{
                      fontSize: 11, color: colors.danger, background: 'none',
                      border: `1px solid ${colors.border}`, borderRadius: radius.sm,
                      padding: '2px 8px', cursor: busy ? 'not-allowed' : 'pointer',
                    }}
                  >
                    Remove
                  </button>
                </span>
              </div>
            );
          })}
          {secrets.truncated && (
            <div style={{ fontSize: 11, color: colors.textDim }}>
              {truncatedNote(secrets.items.length)}
            </div>
          )}
        </div>
      )}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8, maxWidth: 420 }}>
        <TextInput value={name} onChange={setName} placeholder="Secret name" mono disabled={busy} />
        <input
          type="password"
          autoComplete="off"
          value={value}
          disabled={busy}
          placeholder="Value (never shown after save)"
          onChange={e => setValue(e.target.value)}
          aria-label="Secret value"
          style={{
            width: '100%', padding: '8px 12px', background: colors.inputBg,
            border: `1px solid ${colors.border}`, borderRadius: 8, color: colors.text,
            fontFamily: font.mono, fontSize: 12, outline: 'none',
            opacity: busy ? 0.55 : 1,
          }}
        />
        <button
          type="button"
          disabled={busy || !name.trim() || !value.trim()}
          onClick={() => { void write(name.trim(), value); }}
          style={{
            alignSelf: 'flex-start', padding: '7px 14px', borderRadius: radius.sm,
            border: `1px solid ${colors.border}`, background: colors.surface,
            color: colors.cyan, fontSize: 12, fontWeight: 600, fontFamily: font.body,
            cursor: busy ? 'not-allowed' : 'pointer',
          }}
        >
          Set secret
        </button>
      </div>
      {error && <div style={{ marginTop: 8, fontSize: 12, color: colors.danger }}>{error}</div>}
    </div>
  );
}

function GrantsEditor({
  persona,
  capabilities,
  onUpdated,
}: {
  persona: DispatchPersona;
  capabilities: Capability[];
  onUpdated: (p: DispatchPersona) => void;
}) {
  const { colors } = useTheme();
  const enforced = persona.grants_enforced;
  const enabledCaps = capabilities.filter(c => c.enabled);
  const enabledKeys = new Set(enabledCaps.map(c => c.key));
  const [mode, setMode] = useState<GrantMode>(() => grantsToMode(persona.grants));
  const [selected, setSelected] = useState<string[]>(() =>
    persona.grants.mode === 'explicit' ? [...persona.grants.extensions] : [],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const staleKeys = selected.filter(k => !enabledKeys.has(k));
  // The API truncates the grant list it returns, so what is on screen is not the
  // whole set. Saving from a truncated view would silently revoke the grants that
  // were cut off — refuse instead of destroying them.
  const listTruncated = persona.grants.mode === 'explicit' && persona.grants.truncated;

  const save = async () => {
    if (listTruncated) {
      setError('This grant list came back truncated, so saving would drop the grants not shown. Reduce the grant count on disk first.');
      return;
    }
    if (mode === 'narrowed' && staleKeys.length > 0) {
      setError('Stale grants cannot be re-saved until those capabilities are enabled globally.');
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const extensions =
        mode === 'inherit' ? null : mode === 'nothing' ? [] : selected.filter(k => enabledKeys.has(k));
      const updated = await saveGrants(persona.key, extensions);
      onUpdated(updated);
      setMode(grantsToMode(updated.grants));
      setSelected(updated.grants.mode === 'explicit' ? [...updated.grants.extensions] : []);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not save grants');
    } finally {
      setBusy(false);
    }
  };

  const toggle = (key: string) => {
    setSelected(prev => (prev.includes(key) ? prev.filter(k => k !== key) : [...prev, key]));
  };

  return (
    <div>
      {!enforced && (
        <div
          data-testid="grants-not-enforced"
          style={{ fontSize: 12, color: colors.warning, marginBottom: 12, lineHeight: 1.5 }}
        >
          {grantsNotEnforcedNote(persona.engine)}
        </div>
      )}
      <div style={{ fontSize: 12, color: colors.textMuted, marginBottom: 10 }}>
        Current: {grantsSummary(persona)}
      </div>
      {listTruncated && (
        <div style={{ fontSize: 12, color: colors.warning, marginBottom: 10, lineHeight: 1.5 }}>
          This grant list came back truncated, so what is shown is not the whole set. Editing is
          disabled — saving would silently revoke the grants that were cut off.
        </div>
      )}
      <div
        data-testid="grants-editor"
        style={{
          display: 'flex', flexWrap: 'wrap', gap: 8, marginBottom: 12,
          opacity: enforced ? 1 : 0.55, pointerEvents: enforced ? 'auto' : 'none',
        }}
        aria-disabled={!enforced}
      >
        <Chip on={mode === 'inherit'} onClick={() => enforced && setMode('inherit')}>Inherit global</Chip>
        <Chip on={mode === 'nothing'} onClick={() => enforced && setMode('nothing')}>Grant nothing</Chip>
        <Chip on={mode === 'narrowed'} onClick={() => enforced && setMode('narrowed')}>Narrowed</Chip>
      </div>
      {mode === 'narrowed' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginBottom: 12, opacity: enforced ? 1 : 0.55 }}>
          {enabledCaps.map(cap => (
            <label key={cap.key} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12 }}>
              <input
                type="checkbox"
                checked={selected.includes(cap.key)}
                disabled={!enforced || busy}
                onChange={() => toggle(cap.key)}
              />
              <span style={{ color: colors.text }}>{cap.display_name}</span>
              <span style={{ color: colors.textDim, fontFamily: font.mono }}>{cap.key}</span>
            </label>
          ))}
          {staleKeys.map(key => (
            <div
              key={key}
              data-testid={`stale-grant-${key}`}
              style={{ fontSize: 12, color: colors.warning, lineHeight: 1.45, paddingLeft: 2 }}
            >
              Stale grant: <span style={{ fontFamily: font.mono }}>{key}</span> — cannot be re-saved
              until this capability is enabled globally.
            </div>
          ))}
          {enabledCaps.length === 0 && staleKeys.length === 0 && (
            <div style={{ fontSize: 11, color: colors.textDim }}>
              No globally enabled capabilities to grant.
            </div>
          )}
        </div>
      )}
      <button
        type="button"
        disabled={!enforced || busy || listTruncated}
        onClick={() => { void save(); }}
        style={{
          padding: '7px 14px', borderRadius: radius.sm, border: `1px solid ${colors.border}`,
          background: colors.surface, color: enforced ? colors.cyan : colors.textDim,
          cursor: enforced ? 'pointer' : 'not-allowed', fontSize: 12, fontWeight: 600,
          fontFamily: font.body,
        }}
      >
        {busy ? 'Saving…' : 'Save grants'}
      </button>
      {error && <div style={{ marginTop: 8, fontSize: 12, color: colors.danger }}>{error}</div>}
    </div>
  );
}

function WorkSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <div style={{ marginBottom: 18 }}>
      <div style={{ fontSize: 12, fontWeight: 600, color: colors.text, marginBottom: 8 }}>{title}</div>
      {children}
    </div>
  );
}

function renderListUnavailable(reason: string, colors: ReturnType<typeof useTheme>['colors']) {
  return (
    <div style={{ fontSize: 12, color: colors.danger }}>
      Could not be read — {reason}
    </div>
  );
}

function WorkReviewBlock({ work }: { work: WorkReview }) {
  const { colors } = useTheme();
  const activity = work.activity;

  return (
    <div>
      <WorkSection title="Activity">
        {activity.status === 'unavailable'
          ? renderListUnavailable(activity.reason, colors)
          : activity.items.length === 0
            ? <div data-testid="empty-activity" style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>{EMPTY_ACTIVITY_NOTE}</div>
            : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                {activity.items.map(item => (
                  <div key={item.id} style={{ fontSize: 12, color: colors.text, lineHeight: 1.45 }}>
                    <span style={{ color: colors.textDim, fontFamily: font.mono, marginRight: 8 }}>{item.ts}</span>
                    <strong>{item.title}</strong>
                    {item.detail && <span style={{ color: colors.textMuted }}> — {item.detail}</span>}
                  </div>
                ))}
                {activity.truncated && (
                  <div style={{ fontSize: 11, color: colors.textDim }}>{truncatedNote(activity.items.length)}</div>
                )}
              </div>
            )}
      </WorkSection>

      <WorkSection title="Goals">
        {renderGenericSection(work.goals, colors, EMPTY_GOALS_NOTE, goal => {
          const reviews = goal.review_decisions;
          return (
            <div key={goal.id} style={{ fontSize: 12, marginBottom: 8 }}>
              <div style={{ color: colors.text, fontWeight: 500 }}>{goal.title}</div>
              <div style={{ color: colors.textDim }}>{goal.state} · {goal.updated_at}</div>
              {reviews.status === 'unavailable'
                ? renderListUnavailable(reviews.reason, colors)
                : reviews.items.length > 0 && (
                  <div style={{ marginTop: 4, color: colors.textMuted }}>
                    Reviews: {reviews.items.map((d, i) => (
                      <span key={i}>
                        {d.answer ?? '(no answer)'}
                        {d.acted_by ? ` by ${d.acted_by}` : ''}
                        {i < reviews.items.length - 1 ? '; ' : ''}
                      </span>
                    ))}
                    {reviews.truncated && (
                      <div style={{ fontSize: 11 }}>{truncatedNote(reviews.items.length)}</div>
                    )}
                  </div>
                )}
            </div>
          );
        })}
      </WorkSection>

      <WorkSection title="Spend">
        {renderGenericSection(work.spend, colors, EMPTY_SPEND_NOTE, item => (
          <div key={`${item.attribution}-${item.cost_usd}`} style={{ fontSize: 12, color: colors.text }}>
            ${item.cost_usd.toFixed(4)} · {item.call_count} calls
            {item.estimated_call_count > 0 ? ` (${item.estimated_call_count} estimated)` : ''}
            {item.note && <span style={{ color: colors.textMuted }}> — {item.note}</span>}
          </div>
        ))}
      </WorkSection>

      <WorkSection title="Scheduled jobs">
        {renderGenericSection(work.scheduled_jobs, colors, EMPTY_JOBS_NOTE, job => (
          <div key={job.id} style={{ fontSize: 12, color: colors.text, marginBottom: 6 }}>
            <span style={{ fontFamily: font.mono }}>{job.id}</span>
            {' · '}{job.cron}
            {job.paused ? ' · paused' : ''}
            {' · '}runs {job.run_count}
            {job.last_status && ` · last ${job.last_status}`}
            {job.consecutive_failures > 0 && (
              <span style={{ color: colors.danger }}> · {job.consecutive_failures} consecutive failures</span>
            )}
            {job.last_error && <div style={{ color: colors.danger }}>{job.last_error}</div>}
          </div>
        ))}
      </WorkSection>
    </div>
  );
}

function renderGenericSection<T>(
  section: ListSection<T>,
  colors: ReturnType<typeof useTheme>['colors'],
  emptyNote: string,
  renderItem: (item: T) => React.ReactNode,
) {
  if (section.status === 'unavailable') return renderListUnavailable(section.reason, colors);
  if (section.items.length === 0) {
    return <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}>{emptyNote}</div>;
  }
  return (
    <div>
      {section.items.map(renderItem)}
      {section.truncated && (
        <div style={{ fontSize: 11, color: colors.textDim, marginTop: 6 }}>
          {truncatedNote(section.items.length)}
        </div>
      )}
    </div>
  );
}

function AgentDetailPane({
  detail,
  capabilities,
  onBack,
  onPersonaUpdated,
}: {
  detail: AgentDetail;
  capabilities: Capability[];
  onBack: () => void;
  onPersonaUpdated: (p: DispatchPersona) => void;
}) {
  const { colors } = useTheme();
  const [work, setWork] = useState<WorkReview | null>(null);
  const [workError, setWorkError] = useState<string | null>(null);
  const id = detail.kind === 'worker' ? detail.id : detail.key;

  const reloadDetail = useCallback(async () => {
    const next = await fetchAgentDetail(id);
    if (next.kind === 'dispatch_persona') onPersonaUpdated(next);
  }, [id, onPersonaUpdated]);

  useEffect(() => {
    let cancelled = false;
    fetchAgentWork(id)
      .then(w => { if (!cancelled) { setWork(w); setWorkError(null); } })
      .catch(err => {
        if (!cancelled) setWorkError(err instanceof Error ? err.message : 'Could not load work');
      });
    return () => { cancelled = true; };
  }, [id]);

  const headerLive = detail.kind === 'worker'
    ? liveStateLabel(detail.live_state)
    : availabilityLabel(detail.availability);

  return (
    <div>
      <button
        type="button"
        onClick={onBack}
        style={{
          fontSize: 12, color: colors.cyan, background: 'none', border: 'none',
          cursor: 'pointer', padding: 0, marginBottom: 16, fontFamily: font.body, fontWeight: 600,
        }}
      >
        ← Back to agents
      </button>

      <H1 sub={detail.kind === 'worker' ? detail.what_it_does : detail.role}>
        {detail.display_name}
      </H1>

      <div style={{ display: 'flex', flexWrap: 'wrap', gap: '8px 16px', marginBottom: 20, fontSize: 12, color: colors.textMuted }}>
        <span>{detail.kind === 'worker' ? 'Background worker' : 'Dispatch persona'}</span>
        <span>
          {detail.kind === 'worker'
            ? `state source: ${detail.state_source}`
            : `engine: ${engineLabel(detail.engine)}`}
        </span>
        <StatusText text={headerLive.text} tone={headerLive.tone} />
        {detail.kind === 'dispatch_persona' && (
          <span>cost: {detail.cost_tier}</span>
        )}
      </div>

      <Section title="World">
        <WorldLink agentId={id} />
      </Section>

      {detail.kind === 'dispatch_persona' && (
        <>
          <Section
            title="Grants"
            sub="Narrow this agent to a subset of globally enabled capabilities. Empty grant list means grant nothing; inherit means use the global set."
          >
            <GrantsEditor
              persona={detail}
              capabilities={capabilities}
              onUpdated={onPersonaUpdated}
            />
          </Section>
          <Section title="Secrets">
            <SecretsEditor
              agentId={detail.key}
              secrets={detail.secrets}
              onRefresh={reloadDetail}
            />
          </Section>
        </>
      )}

      <Section title="Work review" sub="Rows are exact attribution to this id — empty means nothing is attributed, not that the agent was idle.">
        {workError && <div style={{ fontSize: 12, color: colors.danger }}>{workError}</div>}
        {!workError && !work && <div style={{ fontSize: 12, color: colors.textDim }}>Loading work…</div>}
        {work && <WorkReviewBlock work={work} />}
      </Section>
    </div>
  );
}

export function AgentsPanel({ goto }: PanelProps) {
  const { colors } = useTheme();
  const [roster, setRoster] = useState<RosterResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<AgentDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [unknownFocus, setUnknownFocus] = useState<string | null>(null);

  const pendingAgentFocus = useCommandCenter(s => s.pendingAgentFocus);
  const clearPendingAgentFocus = useCommandCenter(s => s.clearPendingAgentFocus);

  const loadRoster = useCallback(() => {
    fetchRoster()
      // Never hand a non-array to the render path — a degraded daemon reply used
      // to take whole settings panes down with `.map of undefined`.
      .then(r => {
        setRoster({
          workers: Array.isArray(r?.workers) ? r.workers : [],
          dispatch_roster: Array.isArray(r?.dispatch_roster) ? r.dispatch_roster : [],
          capabilities: Array.isArray(r?.capabilities) ? r.capabilities : [],
        });
        setError(null);
      })
      .catch(err => setError(
        `Could not load the agents roster — ${err instanceof Error ? err.message : 'the daemon did not answer'}.`,
      ));
  }, []);

  useEffect(() => { loadRoster(); }, [loadRoster]);

  // Deep-link: open detail for pendingAgentFocus, or land on list with a note.
  useEffect(() => {
    if (!pendingAgentFocus || !roster) return;
    const id = pendingAgentFocus;
    clearPendingAgentFocus();
    const known =
      roster.workers.some(w => w.id === id) ||
      roster.dispatch_roster.some(p => p.key === id);
    if (!known) {
      setUnknownFocus(id);
      setSelectedId(null);
      setDetail(null);
      return;
    }
    setUnknownFocus(null);
    setSelectedId(id);
  }, [pendingAgentFocus, roster, clearPendingAgentFocus]);

  useEffect(() => {
    if (!selectedId) {
      setDetail(null);
      setDetailError(null);
      return;
    }
    let cancelled = false;
    fetchAgentDetail(selectedId)
      .then(d => { if (!cancelled) { setDetail(d); setDetailError(null); } })
      .catch(err => {
        if (!cancelled) {
          setDetail(null);
          setDetailError(err instanceof Error ? err.message : `No agent named ${selectedId}`);
        }
      });
    return () => { cancelled = true; };
  }, [selectedId]);

  if (error && !roster) {
    return <div style={{ color: colors.textMuted, fontSize: 13 }}>{error}</div>;
  }
  if (!roster) {
    return <div style={{ color: colors.textDim, fontSize: 13 }}>Loading agents…</div>;
  }

  if (selectedId && detail) {
    return (
      <AgentDetailPane
        detail={detail}
        capabilities={roster.capabilities}
        onBack={() => { setSelectedId(null); setDetail(null); setDetailError(null); }}
        onPersonaUpdated={p => setDetail({ kind: 'dispatch_persona', ...p })}
      />
    );
  }

  if (selectedId && detailError) {
    return (
      <div>
        <button
          type="button"
          onClick={() => { setSelectedId(null); setDetailError(null); }}
          style={{
            fontSize: 12, color: colors.cyan, background: 'none', border: 'none',
            cursor: 'pointer', padding: 0, marginBottom: 16, fontFamily: font.body, fontWeight: 600,
          }}
        >
          ← Back to agents
        </button>
        <div style={{ fontSize: 13, color: colors.danger }}>
          No agent named {selectedId}. {detailError}
        </div>
      </div>
    );
  }

  return (
    <div>
      <H1 sub="Workers that run themselves, the dispatch roster goals go to, and capabilities with their declared secrets — three populations, kept honest.">
        Agents
      </H1>

      {unknownFocus && (
        <div
          data-testid="unknown-agent"
          style={{
            fontSize: 12, color: colors.warning, marginBottom: 16, lineHeight: 1.5,
            padding: '10px 12px', borderRadius: radius.md, border: `1px solid ${colors.border}`,
          }}
        >
          The roster has no agent named {unknownFocus}. A flag-gated worker — the Guard, for
          instance — is absent from the roster while its flag is off, so this is not proof no
          such agent exists.
        </div>
      )}

      <Section
        title="Workers"
        sub="Background workers run themselves and are not dispatchable."
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {roster.workers.map(worker => (
            <WorkerRow key={worker.id} worker={worker} onOpen={() => setSelectedId(worker.id)} />
          ))}
          {roster.workers.length === 0 && (
            <div style={{ fontSize: 12, color: colors.textDim }}>No background workers visible.</div>
          )}
        </div>
      </Section>

      <Section
        title="Dispatch roster"
        sub="Worker personas goals are dispatched to — engine, availability, cost, and grants."
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {roster.dispatch_roster.map(persona => (
            <PersonaRow key={persona.key} persona={persona} onOpen={() => setSelectedId(persona.key)} />
          ))}
          {roster.dispatch_roster.length === 0 && (
            <div style={{ fontSize: 12, color: colors.textDim }}>No dispatch personas configured.</div>
          )}
        </div>
      </Section>

      <Section
        title="Capabilities"
        sub="Platform extensions — not agents. Enablement is managed under Tools."
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {roster.capabilities.map(cap => (
            <CapabilityRow key={cap.key} capability={cap} onManage={() => goto('tools')} />
          ))}
          {roster.capabilities.length === 0 && (
            <div style={{ fontSize: 12, color: colors.textDim }}>
              The roster listed no capabilities. That is what the daemon returned, not proof none
              are installed.
            </div>
          )}
        </div>
      </Section>
    </div>
  );
}

function WorkerRow({ worker, onOpen }: { worker: BackgroundWorker; onOpen: () => void }) {
  const { colors } = useTheme();
  const live = liveStateLabel(worker.live_state);
  const problem = worker.live_state.status === 'unavailable';
  return (
    <button
      type="button"
      onClick={onOpen}
      data-testid={`worker-row-${worker.id}`}
      style={{
        display: 'block', width: '100%', textAlign: 'left', padding: '12px 14px',
        borderRadius: radius.md, background: colors.bgDeeper,
        border: `1px solid ${problem ? colors.danger : colors.border}`,
        cursor: 'pointer', fontFamily: font.body,
      }}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12, marginBottom: 4 }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{worker.display_name}</span>
        <StatusText text={live.text} tone={live.tone} />
      </div>
      <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.45 }}>{worker.what_it_does}</div>
    </button>
  );
}

function PersonaRow({ persona, onOpen }: { persona: DispatchPersona; onOpen: () => void }) {
  const { colors } = useTheme();
  const avail = availabilityLabel(persona.availability);
  return (
    <button
      type="button"
      onClick={onOpen}
      data-testid={`persona-row-${persona.key}`}
      style={{
        display: 'block', width: '100%', textAlign: 'left', padding: '12px 14px',
        borderRadius: radius.md, background: colors.bgDeeper,
        border: `1px solid ${colors.border}`, cursor: 'pointer', fontFamily: font.body,
      }}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12, marginBottom: 4 }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{persona.display_name}</span>
        <StatusText text={avail.text} tone={avail.tone} />
      </div>
      <div style={{ fontSize: 11, color: colors.textMuted, display: 'flex', flexWrap: 'wrap', gap: '4px 12px' }}>
        <span>{engineLabel(persona.engine)}</span>
        <span>{persona.cost_tier}</span>
      </div>
      <div style={{ fontSize: 11, color: colors.textDim, marginTop: 6 }}>{grantsSummary(persona)}</div>
    </button>
  );
}

function CapabilityRow({
  capability,
  onManage,
}: {
  capability: Capability;
  onManage: () => void;
}) {
  const { colors } = useTheme();
  // Capabilities are not agents — no detail route; deep-link to Tools instead.
  return (
    <div
      data-testid={`capability-row-${capability.key}`}
      style={{
        padding: '12px 14px', borderRadius: radius.md, background: colors.bgDeeper,
        border: `1px solid ${colors.border}`,
      }}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12, marginBottom: 4 }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{capability.display_name}</span>
        <span style={{ fontSize: 11, color: capability.enabled ? colors.cyan : colors.textDim }}>
          {capability.enabled ? 'enabled' : 'disabled'}
        </span>
      </div>
      <div style={{ fontSize: 11, color: colors.textMuted, marginBottom: 4 }}>
        {capability.source} · {defaultEnabledLabel(capability.default_enabled)}
      </div>
      <div style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.45, marginBottom: 8 }}>
        {requiredSecretsLabel(capability.required_secrets)}
      </div>
      <button
        type="button"
        onClick={onManage}
        style={{
          fontSize: 11, color: colors.cyan, background: 'none', border: 'none',
          cursor: 'pointer', padding: 0, fontFamily: font.body, fontWeight: 600,
        }}
      >
        Manage in Tools
      </button>
    </div>
  );
}
