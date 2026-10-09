/**
 * Settings → Agents — three honest populations (workers, dispatch roster,
 * capabilities) over the merged /api/agents surface. No HUD rebuild: the World
 * view owns the HUDs, and this page deep-links into it for the agents that have
 * an in-world character (see lib/worldAgentIds — the two id namespaces differ).
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { Chip, Details, ListRow, Row, Section } from '../atoms';
import { Button } from '../../common/Button';
import { Tooltip } from '../../common/Tooltip';
import { H1 } from '../../common/H1';
import { Toggle } from '../../common/Toggle';
import {
  dispatchAvailabilityLabel,
  defaultEnabledLabel,
  EMPTY_ACTIVITY_NOTE,
  EMPTY_GOALS_NOTE,
  EMPTY_JOBS_NOTE,
  EMPTY_SPEND_NOTE,
  engineLabel,
  gateRowHint,
  grantsNotEnforcedNote,
  grantsSummary,
  liveStatusChip,
  NO_AGENT_SECRETS_NOTE,
  presenceLabel,
  readAgentGate,
  requiredSecretsLabel,
  requiredSecretHints,
  STORED_SECRETS_NOTE,
  truncatedNote,
  workerSummary,
  type AgentGate,
  type LabelTone,
} from '../agentsPanel';
import { font, radius, space, textSize, type } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { api } from '../../../lib/api';
import { useCommandCenter } from '../../../lib/store';
import { timeAgo } from '../format';
import { deJargonDaemonText } from '../../../lib/vocabulary';
import {
  fetchAgentDetail,
  fetchAgentWork,
  fetchRoster,
  saveGrants,
  saveSecret,
  type AgentDetail,
  type BackgroundWorker,
  type LiveState,
  type Capability,
  type DispatchPersona,
  type ListSection,
  type RosterResponse,
  type Secrets,
  type WorkReview,
} from '../../../lib/agentsApi';
import { worldAgentIdForAgent } from '../../../lib/worldAgentIds';
import { ROSTER } from '../../world/agents/roster';
import { AgentPortrait } from './AgentPortrait';
import { AgentSettingsBlock } from './agentSettings';
import { SkillsSection } from './SkillsSection';

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
    <span style={{ fontSize: textSize.micro, color: toneColor(tone, colors), fontWeight: tone === 'error' ? 600 : 400 }}>
      {text}
    </span>
  );
}

/** A worker's live status, narrowed to a plain status or count fact, rendered
 *  as a chip with the raw daemon text (a config key, a failure reason,
 *  whatever it actually said) reachable only via Tooltip, never printed
 *  bare. This is LIVE STATUS, distinct from the on/off `GateChip` beside the
 *  name — the two must never say the same thing in different words. */
function LiveStatusPill({ id, live, gate }: { id: string; live: LiveState; gate: AgentGate | null }) {
  const { colors } = useTheme();
  const chip = liveStatusChip(live, gate);
  return (
    <Tooltip content={chip.detail}>
      <span
        tabIndex={0}
        data-testid={`live-status-${id}`}
        style={{
          ...type.label, padding: `${space.xxs}px ${space.sm}px`, borderRadius: radius.pill,
          border: `1px solid ${colors.border}`, color: toneColor(chip.tone, colors), fontFamily: font.body,
        }}
      >
        {chip.label}
      </span>
    </Tooltip>
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
      <div style={{ fontSize: textSize.caption, color: colors.textDim, lineHeight: 1.5 }}>
        This agent has no in-world presence.
      </div>
    );
  }
  return (
    <div>
      <Button
        colors={colors}
        variant="bare"
        type="button"
        className="hover:underline"
        onClick={() => setUnreachable(!useCommandCenter.getState().focusWorldAgent(worldId))}
        style={{
          '--pa-btn-fg': colors.cyan,
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-pad': '0',
          '--pa-btn-weight': 600,
          fontSize: textSize.caption, fontFamily: font.body,
        } as CSSProperties}
      >
        Open in World
      </Button>
      {unreachable && (
        <div style={{ fontSize: textSize.micro, color: colors.warning, marginTop: space.sm, lineHeight: 1.45 }}>
          No workspace has the World view open, so there is nowhere to fly to. Add it to a
          workspace first.
        </div>
      )}
    </div>
  );
}

/**
 * Three honest states, and no add form.
 *
 * The form used to offer a blank name/value pair with no hint of what belonged
 * in it, and the honest answer to "what am I supposed to put there?" turned out
 * to be *nothing*: `agent_secret.*` has no reader anywhere in the runtime — only
 * this surface writes and lists it. A field whose value nothing consumes is a
 * control that does nothing, so it is gone. Already-stored values stay listed
 * and stay removable; nothing here deletes a secret on its own.
 */
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
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const remove = async (secretName: string) => {
    setBusy(true);
    setError(null);
    try {
      await saveSecret(agentId, secretName, null);
      await onRefresh();
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not remove secret');
      // The catch swallows the throw into `error`, so `false` — the Button
      // contract's failure signal — is what keeps it from ticking anyway.
      return false;
    } finally {
      setBusy(false);
    }
  };

  // A store that could not be READ is never rendered as "needs none" — those are
  // opposite claims, and the failure one is the one the user can act on.
  if (secrets.status === 'unavailable') {
    return (
      <div style={{ fontSize: textSize.caption, color: colors.danger }}>
        Secrets could not be read — {secrets.reason}
      </div>
    );
  }

  if (secrets.items.length === 0) {
    return (
      <div
        data-testid="no-agent-secrets"
        style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}
      >
        {NO_AGENT_SECRETS_NOTE}
      </div>
    );
  }

  return (
    <div>
      <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginBottom: space.lg, lineHeight: 1.5 }}>
        {STORED_SECRETS_NOTE}
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
        {secrets.items.map(item => {
          const label = presenceLabel(item.presence);
          return (
            <div
              key={item.name}
              data-testid={`secret-row-${item.name}`}
              style={{
                display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.xl,
                fontSize: textSize.caption, fontFamily: font.body,
              }}
            >
              <span style={{ color: colors.text, fontFamily: font.mono }}>{item.name}</span>
              <span style={{ display: 'flex', alignItems: 'center', gap: space.lg }}>
                <StatusText text={label.text} tone={label.tone} />
                <Button
                  colors={colors}
                  type="button"
                  disabled={busy}
                  onClick={() => remove(item.name)}
                  style={{
                    '--pa-btn-fg': colors.danger,
                    '--pa-btn-border': colors.border,
                    '--pa-btn-border-hover': `${colors.danger}4D`,
                    '--pa-btn-bg-hover': `${colors.danger}1A`,
                    '--pa-btn-bg-active': `${colors.danger}26`,
                    '--pa-btn-pad': '2px 8px',
                    '--pa-btn-radius': `${radius.sm}px`,
                  } as CSSProperties}
                >
                  Remove
                </Button>
              </span>
            </div>
          );
        })}
        {secrets.truncated && (
          <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
            {truncatedNote(secrets.items.length)}
          </div>
        )}
      </div>
      {error && <div style={{ marginTop: space.md, fontSize: textSize.caption, color: colors.danger }}>{error}</div>}
    </div>
  );
}

/**
 * Rendered ONLY for an engine that actually enforces grants
 * (`WorkerEngineKind::grants_enforced` — today just the internal subagent). The
 * caller makes that decision, because a greyed-out editor still reads as a
 * control you could enable somehow, and there is no somehow: on a pending or CLI
 * engine a saved grant is recorded and enforces nothing.
 */
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
      // Nothing was written. `false` is the Button contract's failure signal:
      // every path here reports through `error`, so without it a refused save
      // would still finish with a success tick.
      return false;
    }
    if (mode === 'narrowed' && staleKeys.length > 0) {
      setError('Stale grants cannot be re-saved until those capabilities are enabled globally.');
      return false;
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
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not save grants');
      return false;
    } finally {
      setBusy(false);
    }
  };

  const toggle = (key: string) => {
    setSelected(prev => (prev.includes(key) ? prev.filter(k => k !== key) : [...prev, key]));
  };

  return (
    <div>
      <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginBottom: space.lg }}>
        Current: {grantsSummary(persona)}
      </div>
      {listTruncated && (
        <div style={{ fontSize: textSize.caption, color: colors.warning, marginBottom: space.lg, lineHeight: 1.5 }}>
          This grant list came back truncated, so what is shown is not the whole set. Editing is
          disabled — saving would silently revoke the grants that were cut off.
        </div>
      )}
      <div
        data-testid="grants-editor"
        style={{ display: 'flex', flexWrap: 'wrap', gap: space.md, marginBottom: space.xl }}
      >
        <Chip on={mode === 'inherit'} onClick={() => setMode('inherit')}>Inherit global</Chip>
        <Chip on={mode === 'nothing'} onClick={() => setMode('nothing')}>Grant nothing</Chip>
        <Chip on={mode === 'narrowed'} onClick={() => setMode('narrowed')}>Narrowed</Chip>
      </div>
      {mode === 'narrowed' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm, marginBottom: space.xl }}>
          {enabledCaps.map(cap => (
            <label key={cap.key} style={{ display: 'flex', alignItems: 'center', gap: space.md, fontSize: textSize.caption }}>
              <input
                type="checkbox"
                checked={selected.includes(cap.key)}
                disabled={busy}
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
              style={{ fontSize: textSize.caption, color: colors.warning, lineHeight: 1.45, paddingLeft: space.xxs }}
            >
              Stale grant: <span style={{ fontFamily: font.mono }}>{key}</span> — cannot be re-saved
              until this capability is enabled globally.
            </div>
          ))}
          {enabledCaps.length === 0 && staleKeys.length === 0 && (
            <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
              No globally enabled capabilities to grant.
            </div>
          )}
        </div>
      )}
      <Button
        colors={colors}
        type="button"
        disabled={busy || listTruncated}
        onClick={() => save()}
        style={{
          '--pa-btn-bg': colors.surface,
          '--pa-btn-fg': listTruncated ? colors.textDim : colors.cyan,
          '--pa-btn-border': colors.border,
          '--pa-btn-border-hover': colors.borderHi,
          '--pa-btn-bg-hover': colors.surfaceHi,
          '--pa-btn-bg-active': colors.surface,
          '--pa-btn-pad': '7px 14px',
          '--pa-btn-radius': `${radius.sm}px`,
          '--pa-btn-weight': 600,
          fontSize: textSize.caption, fontFamily: font.body,
        } as CSSProperties}
      >
        {busy ? 'Saving…' : 'Save grants'}
      </Button>
      {error && <div style={{ marginTop: space.md, fontSize: textSize.caption, color: colors.danger }}>{error}</div>}
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
      <div style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.text, marginBottom: space.md }}>{title}</div>
      {children}
    </div>
  );
}

function renderListUnavailable(reason: string, colors: ReturnType<typeof useTheme>['colors']) {
  return (
    <div style={{ fontSize: textSize.caption, color: colors.danger }}>
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
            ? <div data-testid="empty-activity" style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}>{EMPTY_ACTIVITY_NOTE}</div>
            : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
                {activity.items.map(item => (
                  <div key={item.id} style={{ fontSize: textSize.caption, color: colors.text, lineHeight: 1.45 }}>
                    <span style={{ color: colors.textDim, fontFamily: font.mono, marginRight: space.md }}>{item.ts}</span>
                    <strong>{deJargonDaemonText(item.title)}</strong>
                    {item.detail && <span style={{ color: colors.textMuted }}> — {deJargonDaemonText(item.detail)}</span>}
                  </div>
                ))}
                {activity.truncated && (
                  <div style={{ fontSize: textSize.micro, color: colors.textDim }}>{truncatedNote(activity.items.length)}</div>
                )}
              </div>
            )}
      </WorkSection>

      <WorkSection title="Goals">
        {renderGenericSection(work.goals, colors, EMPTY_GOALS_NOTE, goal => {
          const reviews = goal.review_decisions;
          return (
            <div key={goal.id} style={{ fontSize: textSize.caption, marginBottom: space.md }}>
              <div style={{ color: colors.text, fontWeight: 500 }}>{goal.title}</div>
              <div style={{ color: colors.textDim }}>{goal.state} · {goal.updated_at}</div>
              {reviews.status === 'unavailable'
                ? renderListUnavailable(reviews.reason, colors)
                : reviews.items.length > 0 && (
                  <div style={{ marginTop: space.xs, color: colors.textMuted }}>
                    Reviews: {reviews.items.map((d, i) => (
                      <span key={i}>
                        {d.answer ?? '(no answer)'}
                        {d.acted_by ? ` by ${d.acted_by}` : ''}
                        {i < reviews.items.length - 1 ? '; ' : ''}
                      </span>
                    ))}
                    {reviews.truncated && (
                      <div style={{ fontSize: textSize.micro }}>{truncatedNote(reviews.items.length)}</div>
                    )}
                  </div>
                )}
            </div>
          );
        })}
      </WorkSection>

      <WorkSection title="Spend">
        {renderGenericSection(work.spend, colors, EMPTY_SPEND_NOTE, item => (
          <div key={`${item.attribution}-${item.cost_usd}`} style={{ fontSize: textSize.caption, color: colors.text }}>
            ${item.cost_usd.toFixed(4)} · {item.call_count} calls
            {item.estimated_call_count > 0 ? ` (${item.estimated_call_count} estimated)` : ''}
            {item.note && <span style={{ color: colors.textMuted }}> — {item.note}</span>}
          </div>
        ))}
      </WorkSection>

      <WorkSection title="Scheduled jobs">
        {renderGenericSection(work.scheduled_jobs, colors, EMPTY_JOBS_NOTE, job => (
          <div key={job.id} style={{ fontSize: textSize.caption, color: colors.text, marginBottom: space.sm }}>
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
    return <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}>{emptyNote}</div>;
  }
  return (
    <div>
      {section.items.map(renderItem)}
      {section.truncated && (
        <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.sm }}>
          {truncatedNote(section.items.length)}
        </div>
      )}
    </div>
  );
}

/**
 * The agent's own on/off switch.
 *
 * It writes the SAME config key through the SAME `/config/upsert` route that
 * Settings → Features uses — there is no agent-scoped write path for a gate
 * flag at all. That is what makes "one source of truth" structural instead of a
 * convention: there is no second key for the two surfaces to drift apart on.
 *
 * Optimistic write with revert-on-error, matching FeaturesPanel and the Models
 * pane's Guard block. Three writers, one behaviour.
 */
function AgentEnableRow({
  gate,
  agentName,
  onFlipped,
}: {
  gate: AgentGate;
  agentName: string;
  /** Resolves once the daemon has been re-read, so the guess can be dropped. */
  onFlipped: () => Promise<void>;
}) {
  // The optimistic value is an OVERLAY with a lifetime, not a copy of the gate:
  // an earlier version mirrored `gate.enabled` into state and re-seeded it from
  // a `[gate.enabled]` effect; React skips an effect whose dep did not change,
  // so the one case the re-read exists for — the write returns 200 and the flag
  // is STILL off, which `Config::get_param` produces whenever an env var shadows
  // the config file — left the toggle stuck ON with no error. That overlay is
  // now `Toggle`'s, which was written from this component: it clears when the
  // write settles, so the daemon's answer is what renders, equal or not.
  const [error, setError] = useState<string | null>(null);

  const save = async (v: boolean) => {
    setError(null);
    // A throw is the failure signal — the switch reverts and says so.
    await api.upsertConfig(gate.config_key, v);
    try {
      await onFlipped();
    } catch (err) {
      // The write LANDED; only the read-back failed. Saying "Couldn't save"
      // here would be the same lie in the other direction, so the switch drops
      // back to the last value the daemon actually confirmed and says why.
      setError(`Saved, but could not re-read it: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  return (
    <div data-testid="agent-gate">
      <Section
        title="Enabled"
        sub="One key, written here and under Settings → Features — flipping it in either place is the same flag, and the daemon picks it up at its next tick."
      >
        <Row label={`Enable ${agentName}`} hint={gateRowHint(gate)}>
          <Toggle on={gate.enabled} error={error} onChange={save} label={`Enable ${agentName}`} />
        </Row>
      </Section>
    </div>
  );
}

function AgentDetailPane({
  detail,
  capabilities,
  onBack,
  onPersonaUpdated,
  onDetailReloaded,
  onRosterStale,
}: {
  detail: AgentDetail;
  capabilities: Capability[];
  onBack: () => void;
  onPersonaUpdated: (p: DispatchPersona) => void;
  onDetailReloaded: (d: AgentDetail) => void;
  /** Re-read the list behind this page — its chips are a snapshot of a flag. */
  onRosterStale: () => void;
}) {
  const { colors } = useTheme();
  const [work, setWork] = useState<WorkReview | null>(null);
  const [workError, setWorkError] = useState<string | null>(null);
  const id = detail.kind === 'worker' ? detail.id : detail.key;

  // Both kinds, not just the persona: a worker's re-read is what turns
  // "off (strix_enabled=false)" into its live state after the switch is flipped,
  // and the earlier version dropped that refresh on the floor.
  const reload = useCallback(async () => {
    const next = await fetchAgentDetail(id);
    onDetailReloaded(next);
  }, [id, onDetailReloaded]);

  // The roster is fetched once, on mount, and its on/off chips are a snapshot of
  // the flags AT THAT MOMENT. Flipping a switch here and pressing Back therefore
  // used to land on a list still saying "off" for the agent just switched on —
  // the same "which of these is telling the truth?" confusion this whole change
  // exists to remove, only now with two of our own surfaces disagreeing.
  const reloadAfterFlip = useCallback(async () => {
    await reload();
    onRosterStale();
  }, [reload, onRosterStale]);

  useEffect(() => {
    let cancelled = false;
    fetchAgentWork(id)
      .then(w => { if (!cancelled) { setWork(w); setWorkError(null); } })
      .catch(err => {
        if (!cancelled) setWorkError(err instanceof Error ? err.message : 'Could not load work');
      });
    return () => { cancelled = true; };
  }, [id]);

  // Validated, never cast: an older daemon serialises no gate at all, and a
  // missing gate must read as "no switch known" rather than as a switch that is
  // off — offering to flip a key that daemon does not read would be a lie.
  const gate = readAgentGate(detail);

  const headerAvailability = detail.kind === 'dispatch_persona' ? dispatchAvailabilityLabel(detail) : null;

  return (
    <div>
      <Button
        colors={colors}
        variant="bare"
        type="button"
        className="hover:underline"
        onClick={onBack}
        style={{
          '--pa-btn-fg': colors.cyan,
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-pad': '0',
          '--pa-btn-weight': 600,
          fontSize: textSize.caption, marginBottom: space.xxl, fontFamily: font.body,
        } as CSSProperties}
      >
        ← Back to agents
      </Button>

      <div style={{ display: 'flex', alignItems: 'flex-start', gap: space.xxl }}>
        <AgentPortrait agentId={id} size={56} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <H1 sub={detail.kind === 'worker' ? workerSummary(detail) : detail.role}>
            {detail.display_name}
          </H1>
        </div>
      </div>

      <div style={{ display: 'flex', flexWrap: 'wrap', gap: '8px 16px', marginBottom: space.xxxl, fontSize: textSize.caption, color: colors.textMuted }}>
        <span>{detail.kind === 'worker' ? 'Runs on its own' : 'Takes on goals'}</span>
        {detail.kind === 'dispatch_persona' && <span>{`engine: ${engineLabel(detail.engine)}`}</span>}
        {detail.kind === 'worker'
          ? <LiveStatusPill id={id} live={detail.live_state} gate={gate} />
          : headerAvailability && <StatusText text={headerAvailability.text} tone={headerAvailability.tone} />}
        {detail.kind === 'dispatch_persona' && (
          <span>cost: {detail.cost_tier}</span>
        )}
      </div>

      {detail.kind === 'worker' && (
        <Details summary="How this works">
          <div style={{ marginBottom: space.md }}>{detail.what_it_does}</div>
          <div>{detail.why_it_matters}</div>
        </Details>
      )}

      {gate && (
        <AgentEnableRow
          gate={gate}
          agentName={detail.display_name}
          onFlipped={reloadAfterFlip}
        />
      )}

      {/* This agent's own settings, which used to sit under Models because
          they each name a model (J8/C7). Models keeps "which brain answers
          which job"; how an agent behaves belongs to the agent. */}
      <AgentSettingsBlock agentId={id} />

      <Section title="World">
        <WorldLink agentId={id} />
      </Section>

      {detail.kind === 'dispatch_persona' && (
        <>
          <Section
            title="Grants"
            sub={detail.grants_enforced
              ? 'Narrow this agent to a subset of globally enabled capabilities. Empty grant list means grant nothing; inherit means use the global set.'
              : 'What this agent is recorded as granted. This engine does not enforce grants, so there is nothing here to narrow.'}
          >
            {detail.grants_enforced ? (
              <GrantsEditor
                persona={detail}
                capabilities={capabilities}
                onUpdated={onPersonaUpdated}
              />
            ) : (
              // No chips, no checkboxes, no Save. An editor whose engine ignores
              // what it saves is a control that does nothing; the read-only line
              // still shows what is recorded on disk, and the sentence says why
              // it cannot be edited here.
              <div>
                <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginBottom: space.lg }}>
                  Current: {grantsSummary(detail)}
                </div>
                <div
                  data-testid="grants-not-enforced"
                  style={{ fontSize: textSize.caption, color: colors.warning, lineHeight: 1.5 }}
                >
                  {grantsNotEnforcedNote(detail.engine)}
                </div>
              </div>
            )}
          </Section>
          <Section title="Secrets">
            <SecretsEditor
              agentId={detail.key}
              secrets={detail.secrets}
              onRefresh={reload}
            />
          </Section>
        </>
      )}

      <Section title="Work review" sub="Rows are exact attribution to this id — empty means nothing is attributed, not that the agent was idle.">
        {workError && <div style={{ fontSize: textSize.caption, color: colors.danger }}>{workError}</div>}
        {!workError && !work && <div style={{ fontSize: textSize.caption, color: colors.textDim }}>Loading work…</div>}
        {work && <WorkReviewBlock work={work} />}
      </Section>
    </div>
  );
}

/** One worker's most recent attributed activity, for its roster row. Three
 *  DISTINCT states (R2/R5): `ok` (a real item, timestamped and titled), `none`
 *  (the read succeeded and found nothing attributed — brief ST1's "say what's
 *  available or say what's missing"), and `unavailable` (the read itself
 *  failed — never collapsed into `none`, or a dead read would read as a quiet
 *  agent). */
export type RecentActivityState =
  | { status: 'ok'; ts: string; title: string }
  | { status: 'none' }
  | { status: 'unavailable' };

export function AgentsPanel({ goto }: PanelProps) {
  const { colors } = useTheme();
  const [roster, setRoster] = useState<RosterResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<AgentDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [unknownFocus, setUnknownFocus] = useState<string | null>(null);
  // Keyed by worker id. `undefined` = not yet loaded (roster rows render with
  // no activity line rather than a permanent "loading" — the request is
  // cheap and near-instant, and a skeleton here would outlive the fetch on
  // most daemons).
  const [recentActivity, setRecentActivity] = useState<Record<string, RecentActivityState>>({});

  const pendingAgentFocus = useCommandCenter(s => s.pendingAgentFocus);
  const clearPendingAgentFocus = useCommandCenter(s => s.clearPendingAgentFocus);

  // One cheap, capped read per worker — brief ST1: every roster row needs its
  // last activity, not just the one the user happens to click into. Each
  // worker's read fails independently (Promise.allSettled), so one dead
  // worker cannot blank the other twelve rows.
  const loadRecentActivity = useCallback((workers: BackgroundWorker[]) => {
    void Promise.allSettled(workers.map(w => fetchAgentWork(w.id, 1))).then(results => {
      setRecentActivity(prev => {
        const next = { ...prev };
        results.forEach((r, i) => {
          const id = workers[i].id;
          if (r.status !== 'fulfilled') { next[id] = { status: 'unavailable' }; return; }
          const activity = r.value.activity;
          if (activity.status === 'unavailable') { next[id] = { status: 'unavailable' }; return; }
          const item = activity.items[0];
          next[id] = item ? { status: 'ok', ts: item.ts, title: deJargonDaemonText(item.title) } : { status: 'none' };
        });
        return next;
      });
    });
  }, []);

  const loadRoster = useCallback(() => {
    fetchRoster()
      // Never hand a non-array to the render path — a degraded daemon reply used
      // to take whole settings panes down with `.map of undefined`.
      .then(r => {
        const workers = Array.isArray(r?.workers) ? r.workers : [];
        setRoster({
          workers,
          dispatch_roster: Array.isArray(r?.dispatch_roster) ? r.dispatch_roster : [],
          capabilities: Array.isArray(r?.capabilities) ? r.capabilities : [],
        });
        setError(null);
        loadRecentActivity(workers);
      })
      .catch(err => setError(
        `Could not load the agents roster — ${err instanceof Error ? err.message : 'the daemon did not answer'}.`,
      ));
  }, [loadRecentActivity]);

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
    return <div style={{ color: colors.textMuted, fontSize: textSize.small }}>{error}</div>;
  }
  if (!roster) {
    return <div style={{ color: colors.textDim, fontSize: textSize.small }}>Loading agents…</div>;
  }

  if (selectedId && detail) {
    return (
      <AgentDetailPane
        detail={detail}
        capabilities={roster.capabilities}
        onBack={() => { setSelectedId(null); setDetail(null); setDetailError(null); }}
        onPersonaUpdated={p => setDetail({ kind: 'dispatch_persona', ...p })}
        onDetailReloaded={d => setDetail(d)}
        onRosterStale={loadRoster}
      />
    );
  }

  if (selectedId && detailError) {
    return (
      <div>
        <Button
          colors={colors}
          variant="bare"
          type="button"
          className="hover:underline"
          onClick={() => { setSelectedId(null); setDetailError(null); }}
          style={{
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-weight': 600,
            fontSize: textSize.caption, marginBottom: space.xxl, fontFamily: font.body,
          } as CSSProperties}
        >
          ← Back to agents
        </Button>
        <div style={{ fontSize: textSize.small, color: colors.danger }}>
          No agent named {selectedId}. {detailError}
        </div>
      </div>
    );
  }

  return (
    <div>
      <H1 sub="Agents that work on their own, agents that take on your goals, and what they're all allowed to use. A switched-off agent still shows up here — its switch is on this page.">
        Agents
      </H1>

      {unknownFocus && (
        <div
          data-testid="unknown-agent"
          style={{
            fontSize: textSize.caption, color: colors.warning, marginBottom: space.xxl, lineHeight: 1.5,
            padding: `${space.lg}px ${space.xl}px`, borderRadius: radius.md, border: `1px solid ${colors.border}`,
          }}
        >
          The roster the daemon returned has no agent named {unknownFocus}. Flag-gated workers
          are listed here even while their flag is off, so being switched off is no longer an
          explanation — a daemon that answered with a partial roster still would be.
        </div>
      )}

      <Section
        title="Background agents"
        sub="They run on their own — you don't send them a goal. Switched-off agents stay listed so you can turn them on."
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
          {roster.workers.map(worker => (
            <WorkerRow
              key={worker.id}
              worker={worker}
              activity={recentActivity[worker.id]}
              onOpen={() => setSelectedId(worker.id)}
            />
          ))}
          {roster.workers.length === 0 && (
            <div style={{ fontSize: textSize.caption, color: colors.textDim }}>No background workers visible.</div>
          )}
        </div>
      </Section>

      <Section
        title="Agents that take on goals"
        sub="You (or the app) hand these a goal to work on. Shows what's running it, whether it's ready, and what it costs."
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
          {roster.dispatch_roster.map(persona => (
            <PersonaRow key={persona.key} persona={persona} onOpen={() => setSelectedId(persona.key)} />
          ))}
          {roster.dispatch_roster.length === 0 && (
            <div style={{ fontSize: textSize.caption, color: colors.textDim }}>No dispatch personas configured.</div>
          )}
        </div>
      </Section>

      {/* Skills sit with the agents because they are what the agents learned —
          the ruled placement (J4). Before this the Library had no front door at
          all: you could only arrive by accepting a proposal or by clicking a
          skill inside another tab's condensed list. */}
      <SkillsSection />

      <Section
        title="What your agents can use"
        sub="Tools your agents can call. Review their configured status in Tools."
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
          {roster.capabilities.map(cap => (
            <CapabilityRow key={cap.key} capability={cap} onManage={() => goto('tools')} />
          ))}
          {roster.capabilities.length === 0 && (
            <div style={{ fontSize: textSize.caption, color: colors.textDim }}>
              Nothing came back from the daemon just now — that's not proof none are installed.
            </div>
          )}
        </div>
      </Section>
    </div>
  );
}

/**
 * The on/off chip is why a gated worker is LISTED rather than hidden: the list
 * itself now answers "is this thing switched on?", which is the question that
 * previously sent the user hunting through five panes for a control they could
 * not find because the row it belonged to was filtered away.
 */
/** Every row shows its on/off state the same way, in the same place — ruling
 *  2026-09-25's 1440px review: Scheduler, the Watcher and the Financier had no gate chip
 *  at all, so "switched on" and "has no switch" looked identical (blank).
 *  `gate: null` means the worker has no switch to flip, which is a fact worth
 *  saying — "Always on" — not a reason to render nothing. This is the ONE
 *  place on/off is shown; the live-status chip beside it never repeats it
 *  (see `liveStatusChip`, which narrows a bare "on"/"off" daemon value to a
 *  different word — Running/Idle — for exactly that reason). */
function GateChip({ id, gate }: { id: string; gate: AgentGate | null }) {
  const { colors } = useTheme();
  if (!gate) {
    return (
      <span
        data-testid={`gate-chip-${id}`}
        style={{
          ...type.label, padding: `${space.xxs}px ${space.sm}px`, borderRadius: radius.pill,
          border: `1px solid ${colors.border}`, color: colors.textDim, fontFamily: font.body,
        }}
      >
        Always on
      </span>
    );
  }
  return (
    <span
      data-testid={`gate-chip-${id}`}
      style={{
        ...type.label, padding: `${space.xxs}px ${space.sm}px`, borderRadius: radius.pill,
        border: `1px solid ${gate.enabled ? colors.borderHi : colors.border}`,
        color: gate.enabled ? colors.cyan : colors.textDim,
        fontFamily: font.body,
      }}
    >
      {gate.enabled ? 'on' : 'off'}
    </span>
  );
}

/** The row's "last activity" line — brief ST1's third and fourth asks: show
 *  what's available, or say plainly what's missing, never leave it blank as
 *  though there were nothing to say. `undefined` (not yet loaded) renders
 *  nothing, on purpose — see the `recentActivity` state's own comment. */
function RecentActivityLine({ activity }: { activity: RecentActivityState | undefined }) {
  const { colors } = useTheme();
  if (!activity) return null;
  if (activity.status === 'unavailable') {
    return <span style={{ color: colors.textDim }}>Recent work — couldn&apos;t be read just now</span>;
  }
  if (activity.status === 'none') {
    return <span style={{ color: colors.textDim }}>No activity attributed to this agent yet</span>;
  }
  return (
    <span style={{ color: colors.textDim }}>
      Last activity {timeAgo(activity.ts) || activity.ts} — {activity.title}
    </span>
  );
}

function WorkerRow({ worker, activity, onOpen }: {
  worker: BackgroundWorker;
  activity: RecentActivityState | undefined;
  onOpen: () => void;
}) {
  const problem = worker.live_state.status === 'unavailable';
  const gate = readAgentGate(worker);
  return (
    <ListRow
      testId={`worker-row-${worker.id}`}
      onOpen={onOpen}
      tone={problem ? 'problem' : 'default'}
      icon={<AgentPortrait agentId={worker.id} size={28} />}
      title={worker.display_name}
      chip={<GateChip id={worker.id} gate={gate} />}
      status={<LiveStatusPill id={worker.id} live={worker.live_state} gate={gate} />}
      detail={
        <>
          <div>{workerSummary(worker)}</div>
          <div style={{ fontSize: textSize.micro, marginTop: space.xs }} data-testid={`worker-row-${worker.id}-activity`}>
            <RecentActivityLine activity={activity} />
          </div>
        </>
      }
    />
  );
}

function PersonaRow({ persona, onOpen }: { persona: DispatchPersona; onOpen: () => void }) {
  const avail = dispatchAvailabilityLabel(persona);
  const gate = readAgentGate(persona);
  return (
    <ListRow
      testId={`persona-row-${persona.key}`}
      onOpen={onOpen}
      icon={<AgentPortrait agentId={persona.key} size={28} />}
      title={persona.display_name}
      chip={<GateChip id={persona.key} gate={gate} />}
      status={<StatusText text={avail.text} tone={avail.tone} />}
      detail={
        <>
          <div style={{ fontSize: textSize.micro, display: 'flex', flexWrap: 'wrap', gap: '4px 12px' }}>
            <span>{engineLabel(persona.engine)}</span>
            <span>{persona.cost_tier}</span>
          </div>
          <div style={{ fontSize: textSize.micro, marginTop: space.xs }}>{grantsSummary(persona)}</div>
        </>
      }
    />
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
    <ListRow
      testId={`capability-row-${capability.key}`}
      title={capability.display_name}
      status={
        <span style={{ fontSize: textSize.micro, color: capability.enabled ? colors.cyan : colors.textDim }}>
          {capability.enabled ? 'enabled' : 'disabled'}
        </span>
      }
      detail={
        <>
          <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginBottom: space.xs }}>
            {capability.source} · {defaultEnabledLabel(capability.default_enabled)}
          </div>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.45 }}>
            {requiredSecretsLabel(capability.required_secrets)}
            {requiredSecretHints(capability.required_secrets).map(hint => (
              <div key={hint} data-testid="required-secret-hint" style={{ marginTop: space.xxs }}>{hint}</div>
            ))}
          </div>
        </>
      }
      trailing={
        <Button
          colors={colors}
          variant="bare"
          type="button"
          className="hover:underline"
          onClick={onManage}
          style={{
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-weight': 600,
            fontFamily: font.body,
          } as CSSProperties}
        >
          Manage in Tools
        </Button>
      }
    />
  );
}
