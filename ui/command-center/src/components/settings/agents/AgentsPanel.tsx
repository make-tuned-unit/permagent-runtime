/**
 * Settings → Agents — three honest populations (workers, dispatch roster,
 * capabilities) over the merged /api/agents surface. No HUD rebuild: the World
 * view owns the HUDs, and this page deep-links into it for the agents that have
 * an in-world character (see lib/worldAgentIds — the two id namespaces differ).
 */

import { useCallback, useEffect, useState } from 'react';
import { Chip, H1, Row, Section, Toggle } from '../atoms';
import {
  availabilityLabel,
  defaultEnabledLabel,
  EMPTY_ACTIVITY_NOTE,
  EMPTY_GOALS_NOTE,
  EMPTY_JOBS_NOTE,
  EMPTY_SPEND_NOTE,
  engineLabel,
  gateRowHint,
  grantsNotEnforcedNote,
  grantsSummary,
  liveStateLabel,
  NO_AGENT_SECRETS_NOTE,
  presenceLabel,
  readAgentGate,
  requiredSecretsLabel,
  requiredSecretHints,
  STORED_SECRETS_NOTE,
  truncatedNote,
  type AgentGate,
  type LabelTone,
} from '../agentsPanel';
import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { api } from '../../../lib/api';
import { useCommandCenter } from '../../../lib/store';
import {
  askAgent,
  fetchAgentDetail,
  fetchAgentWork,
  fetchRoster,
  readAgentCapability,
  runAgentNow,
  saveGrants,
  saveSecret,
  type AgentCapability,
  type AgentDetail,
  type AgentRun,
  type AppliedToolScope,
  type AskAnswer,
  type BackgroundWorker,
  type BriefingItem,
  type Capability,
  type DispatchPersona,
  type ListSection,
  type RosterResponse,
  type RunOutcome,
  type RunsSection,
  type Secrets,
  type WorkReview,
} from '../../../lib/agentsApi';
import { worldAgentIdForAgent } from '../../../lib/worldAgentIds';
import { ROSTER } from '../../world/agents/roster';
import { AgentPortrait } from './AgentPortrait';

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
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not remove secret');
    } finally {
      setBusy(false);
    }
  };

  // A store that could not be READ is never rendered as "needs none" — those are
  // opposite claims, and the failure one is the one the user can act on.
  if (secrets.status === 'unavailable') {
    return (
      <div style={{ fontSize: 12, color: colors.danger }}>
        Secrets could not be read — {secrets.reason}
      </div>
    );
  }

  if (secrets.items.length === 0) {
    return (
      <div
        data-testid="no-agent-secrets"
        style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}
      >
        {NO_AGENT_SECRETS_NOTE}
      </div>
    );
  }

  return (
    <div>
      <div style={{ fontSize: 12, color: colors.textMuted, marginBottom: 10, lineHeight: 1.5 }}>
        {STORED_SECRETS_NOTE}
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
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
                  onClick={() => { void remove(item.name); }}
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
      {error && <div style={{ marginTop: 8, fontSize: 12, color: colors.danger }}>{error}</div>}
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
        style={{ display: 'flex', flexWrap: 'wrap', gap: 8, marginBottom: 12 }}
      >
        <Chip on={mode === 'inherit'} onClick={() => setMode('inherit')}>Inherit global</Chip>
        <Chip on={mode === 'nothing'} onClick={() => setMode('nothing')}>Grant nothing</Chip>
        <Chip on={mode === 'narrowed'} onClick={() => setMode('narrowed')}>Narrowed</Chip>
      </div>
      {mode === 'narrowed' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginBottom: 12 }}>
          {enabledCaps.map(cap => (
            <label key={cap.key} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12 }}>
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
        disabled={busy || listTruncated}
        onClick={() => { void save(); }}
        style={{
          padding: '7px 14px', borderRadius: radius.sm, border: `1px solid ${colors.border}`,
          background: colors.surface, color: listTruncated ? colors.textDim : colors.cyan,
          cursor: busy || listTruncated ? 'not-allowed' : 'pointer', fontSize: 12, fontWeight: 600,
          fontFamily: font.body,
        }}
      >
        {busy ? 'Saving…' : 'Save grants'}
      </button>
      {error && <div style={{ marginTop: 8, fontSize: 12, color: colors.danger }}>{error}</div>}
    </div>
  );
}

/**
 * What the answer's tools actually were, in a sentence.
 *
 * `inherit_global` is deliberately NOT described as "its own tools": the turn
 * carried the globally enabled set, the same set any other in-process run would
 * get, and calling that the agent's own scope would overstate what happened.
 * `explicit` names the grants that were declared but not available, because a
 * grant that narrowed to nothing is invisible otherwise.
 */
function toolScopeLabel(scope: AppliedToolScope): string {
  if (scope.mode === 'inherit_global') {
    return scope.extensions.length === 0
      ? 'the globally enabled tools, of which there are currently none'
      : `the globally enabled tools (${scope.extensions.join(', ')}) — not a set of its own`;
  }
  const missing = scope.granted.filter(g => !scope.applied.includes(g));
  const base = scope.applied.length === 0
    ? 'narrowed to its own grants, none of which were available'
    : `narrowed to its own grants: ${scope.applied.join(', ')}`;
  return missing.length === 0
    ? base
    : `${base} (declared but not available: ${missing.join(', ')})`;
}

/**
 * The first question this page exists to answer: "can I actually talk to this
 * one?". It is a request/response ask against THIS agent's id — the answer
 * comes back whole, and the identity and tool scope it came back under are
 * printed beside it, which is the whole difference between this and a generic
 * chat box that would answer as nobody in particular.
 */
function AskAgentBlock({
  agentId,
  agentName,
  ask,
}: {
  agentId: string;
  agentName: string;
  ask: AgentCapability;
}) {
  const { colors } = useTheme();
  const [question, setQuestion] = useState('');
  const [busy, setBusy] = useState(false);
  const [answer, setAnswer] = useState<AskAnswer | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Disabled and VISIBLE, never hidden: the user needs to see that asking is a
  // thing this surface does, and why it is off for this agent.
  const blocked = ask.status === 'unavailable';
  const canSend = !blocked && !busy && question.trim().length > 0;

  const send = async () => {
    if (!canSend) return;
    setBusy(true);
    setError(null);
    setAnswer(null);
    try {
      setAnswer(await askAgent(agentId, question.trim()));
    } catch (err) {
      setError(err instanceof Error ? err.message : 'The agent did not answer');
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section
      title="Ask this agent"
      sub="Asks this agent, under its own persona and tool scope. The answer arrives whole when it is ready — there is nothing to watch while it thinks."
    >
      {blocked && (
        <div
          data-testid="ask-unavailable"
          style={{ fontSize: 12, color: colors.warning, marginBottom: 10, lineHeight: 1.5 }}
        >
          {agentName} cannot be asked anything here — {ask.reason}
        </div>
      )}
      <textarea
        data-testid="ask-question"
        value={question}
        disabled={blocked || busy}
        onChange={e => setQuestion(e.target.value)}
        placeholder={blocked ? '' : `Ask ${agentName} a question`}
        style={{
          width: '100%', padding: 12, background: colors.inputBg,
          border: `1px solid ${colors.border}`, borderRadius: 8, color: colors.text,
          fontFamily: font.body, fontSize: 13, outline: 'none', minHeight: 80,
          resize: 'vertical', cursor: blocked || busy ? 'not-allowed' : 'text',
          opacity: blocked || busy ? 0.55 : 1,
        }}
      />
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginTop: 10 }}>
        <button
          type="button"
          data-testid="ask-send"
          disabled={!canSend}
          onClick={() => { void send(); }}
          style={{
            padding: '7px 14px', borderRadius: radius.sm, border: `1px solid ${colors.border}`,
            background: colors.surface, color: canSend ? colors.cyan : colors.textDim,
            cursor: canSend ? 'pointer' : 'not-allowed', fontSize: 12, fontWeight: 600,
            fontFamily: font.body,
          }}
        >
          {busy ? 'Asking…' : 'Ask'}
        </button>
        {busy && (
          <span data-testid="ask-pending" style={{ fontSize: 12, color: colors.textMuted }}>
            Waiting for {agentName} to answer…
          </span>
        )}
      </div>
      {error && (
        <div data-testid="ask-error" style={{ marginTop: 10, fontSize: 12, color: colors.danger }}>
          Could not ask {agentName} — {error}
        </div>
      )}
      {answer && (
        <div style={{ marginTop: 14 }}>
          <div
            data-testid="ask-answer-attribution"
            style={{ fontSize: 11, color: colors.textDim, marginBottom: 6, lineHeight: 1.5 }}
          >
            Answered by {answer.display_name} · tools: {toolScopeLabel(answer.tool_scope)} ·{' '}
            {answer.persona_applied
              ? 'its own persona was applied'
              : 'answered WITHOUT its persona'}
          </div>
          <div
            data-testid="ask-answer"
            style={{
              fontSize: 12, color: colors.text, lineHeight: 1.55, whiteSpace: 'pre-wrap',
              padding: '10px 12px', borderRadius: radius.md,
              background: colors.bgDeeper, border: `1px solid ${colors.border}`,
            }}
          >
            {answer.answer}
          </div>
        </div>
      )}
    </Section>
  );
}

function WorkSection({
  title,
  action,
  children,
}: {
  title: string;
  /** Sits on the header line, right-aligned — where "Run now" lives, next to
   *  the Runs list it writes into. */
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <div style={{ marginBottom: 18 }}>
      <div
        style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          gap: 12, marginBottom: 8, minHeight: 20,
        }}
      >
        <div style={{ fontSize: 12, fontWeight: 600, color: colors.text }}>{title}</div>
        {action}
      </div>
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

/**
 * "No runs recorded YET" — a fact about the record, from an agent that does
 * keep one. Deliberately different words from the not-recorded line below, which
 * is a fact about the agent.
 */
const EMPTY_RUNS_NOTE =
  'No runs recorded yet. This agent does record its passes, so one will appear here after its next.';

const EMPTY_BRIEFINGS_NOTE =
  'No briefings from this agent. Absence here is not proof it had nothing to say — an agent that files no briefings reports through its activity instead.';

/**
 * `ok` and `skipped` are both the agent working as designed — a skipped pass
 * means nothing was due, and colouring it as a failure is exactly the false
 * alarm this page exists to stop. Only `failed` gets the error treatment.
 */
function runOutcomeTone(outcome: RunOutcome): LabelTone {
  if (outcome === 'failed') return 'error';
  if (outcome === 'ok') return 'ok';
  return 'unknown';
}

/**
 * One recorded pass. Every optional field is rendered ONLY when the daemon sent
 * one: `examined: null` means this pass does not count items, and printing it
 * as 0 would be a claim that the agent looked at nothing.
 */
function RunRow({ run }: { run: AgentRun }) {
  const { colors } = useTheme();
  const tone = runOutcomeTone(run.outcome);
  return (
    <div
      data-testid={`run-row-${run.id}`}
      style={{ fontSize: 12, color: colors.text, lineHeight: 1.45 }}
    >
      <span style={{ color: colors.textDim, fontFamily: font.mono, marginRight: 8 }}>
        {run.started_at}
      </span>
      <span style={{ color: colors.textMuted, marginRight: 8 }}>{run.trigger}</span>
      <span
        data-testid={`run-outcome-${run.id}`}
        data-tone={tone}
        style={{ color: toneColor(tone, colors), fontWeight: tone === 'error' ? 600 : 400 }}
      >
        {run.outcome}
      </span>
      {run.examined !== null && (
        <span style={{ color: colors.textMuted }}> · examined {run.examined}</span>
      )}
      {run.produced !== null && (
        <span style={{ color: colors.textMuted }}> · produced {run.produced}</span>
      )}
      {run.reason !== null && (
        <div style={{ color: tone === 'error' ? colors.danger : colors.textMuted, marginTop: 2 }}>
          {run.reason}
        </div>
      )}
    </div>
  );
}

/**
 * Runs, and the one control that adds to them.
 *
 * "Run now" is a REQUEST/RESPONSE call: it resolves when the pass has finished
 * and the runtime has written its row. There is no progress stream behind it,
 * so nothing here says live, streaming, or watching — the honest promise is
 * "press it, wait, see the run it recorded".
 */
function RunsBlock({
  agentId,
  runs,
  runNow,
  onRan,
}: {
  agentId: string;
  runs: RunsSection;
  runNow: AgentCapability;
  /** Re-read the work review, so the pass just run joins the list. */
  onRan: () => Promise<void>;
}) {
  const { colors } = useTheme();
  const [busy, setBusy] = useState(false);
  const [lastRun, setLastRun] = useState<AgentRun | null>(null);
  const [error, setError] = useState<string | null>(null);

  const blocked = runNow.status === 'unavailable';

  const go = async () => {
    setBusy(true);
    setError(null);
    try {
      const { run } = await runAgentNow(agentId);
      setLastRun(run);
      // The list on screen is a snapshot taken before this pass existed.
      await onRan();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'The run did not complete');
    } finally {
      setBusy(false);
    }
  };

  const button = (
    <button
      type="button"
      data-testid="run-now"
      disabled={blocked || busy}
      onClick={() => { void go(); }}
      style={{
        padding: '4px 10px', borderRadius: radius.sm, border: `1px solid ${colors.border}`,
        background: colors.surface, color: blocked || busy ? colors.textDim : colors.cyan,
        cursor: blocked || busy ? 'not-allowed' : 'pointer', fontSize: 11, fontWeight: 600,
        fontFamily: font.body,
      }}
    >
      {busy ? 'Running…' : 'Run now'}
    </button>
  );

  return (
    <WorkSection title="Runs" action={button}>
      {blocked && (
        <div
          data-testid="run-now-unavailable"
          style={{ fontSize: 12, color: colors.warning, marginBottom: 8, lineHeight: 1.5 }}
        >
          This agent cannot be run from here — {runNow.reason}
        </div>
      )}
      {busy && (
        <div data-testid="run-now-pending" style={{ fontSize: 12, color: colors.textMuted, marginBottom: 8 }}>
          Running one pass. This returns the recorded run when the pass finishes.
        </div>
      )}
      {error && (
        <div data-testid="run-now-error" style={{ fontSize: 12, color: colors.danger, marginBottom: 8 }}>
          Could not run it — {error}
        </div>
      )}
      {lastRun && (
        <div
          data-testid="run-now-result"
          style={{
            marginBottom: 10, padding: '8px 10px', borderRadius: radius.md,
            background: colors.bgDeeper, border: `1px solid ${colors.border}`,
          }}
        >
          <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 4 }}>
            The pass you just ran:
          </div>
          <RunRow run={lastRun} />
        </div>
      )}
      {runs.status === 'unavailable' && renderListUnavailable(runs.reason, colors)}
      {runs.status === 'not_recorded' && (
        // NOT an empty list and NOT an error: this agent's code writes no run
        // record at all, so there is nothing here to be missing.
        <div
          data-testid="runs-not-recorded"
          style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}
        >
          This agent records no runs — {runs.reason}
        </div>
      )}
      {runs.status === 'ok' && runs.items.length === 0 && (
        <div
          data-testid="empty-runs"
          style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}
        >
          {EMPTY_RUNS_NOTE}
        </div>
      )}
      {runs.status === 'ok' && runs.items.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {runs.items.map(run => <RunRow key={run.id} run={run} />)}
          {runs.truncated && (
            <div style={{ fontSize: 11, color: colors.textDim }}>{truncatedNote(runs.items.length)}</div>
          )}
        </div>
      )}
    </WorkSection>
  );
}

function briefingSeverityColor(
  severity: BriefingItem['severity'],
  colors: ReturnType<typeof useTheme>['colors'],
): string {
  // A briefing asking for a decision is not a failure — it is warning-toned, so
  // it cannot be confused with a run that went wrong.
  if (severity === 'action_required' || severity === 'attention') return colors.warning;
  return colors.textMuted;
}

function BriefingsBlock({ briefings }: { briefings: ListSection<BriefingItem> }) {
  const { colors } = useTheme();
  if (briefings.status === 'unavailable') {
    return (
      <WorkSection title="Briefings">
        {renderListUnavailable(briefings.reason, colors)}
      </WorkSection>
    );
  }
  return (
    <WorkSection title="Briefings">
      {briefings.items.length === 0 ? (
        <div
          data-testid="empty-briefings"
          style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.5 }}
        >
          {EMPTY_BRIEFINGS_NOTE}
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {briefings.items.map(item => (
            <div
              key={item.id}
              data-testid={`briefing-row-${item.id}`}
              style={{ fontSize: 12, color: colors.text, lineHeight: 1.45 }}
            >
              <span
                style={{
                  color: briefingSeverityColor(item.severity, colors),
                  fontWeight: item.severity === 'action_required' ? 600 : 400,
                  marginRight: 8,
                }}
              >
                {item.severity}
              </span>
              <span>{item.summary}</span>
              <div style={{ color: colors.textDim, fontSize: 11, marginTop: 2 }}>
                <span style={{ fontFamily: font.mono, marginRight: 8 }}>{item.created_at}</span>
                {item.acknowledged_at === null
                  ? 'not acknowledged'
                  : `acknowledged ${item.acknowledged_at}`}
              </div>
              {item.detail && (
                <div style={{ color: colors.textMuted, marginTop: 2 }}>{item.detail}</div>
              )}
            </div>
          ))}
          {briefings.truncated && (
            <div style={{ fontSize: 11, color: colors.textDim }}>
              {truncatedNote(briefings.items.length)}
            </div>
          )}
        </div>
      )}
    </WorkSection>
  );
}

function WorkReviewBlock({
  work,
  agentId,
  runNow,
  onRan,
}: {
  work: WorkReview;
  agentId: string;
  runNow: AgentCapability;
  onRan: () => Promise<void>;
}) {
  const { colors } = useTheme();
  const activity = work.activity;

  return (
    <div>
      {/* Runs first: "did it actually do the thing" is the question this whole
          section is here to answer, and everything below is context for it. */}
      <RunsBlock agentId={agentId} runs={work.runs} runNow={runNow} onRan={onRan} />

      <BriefingsBlock briefings={work.briefings} />

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
  const { colors } = useTheme();
  // The optimistic value is an OVERLAY with a lifetime, not a copy of the gate.
  // An earlier version mirrored `gate.enabled` into state and re-seeded it from
  // a `[gate.enabled]` effect; React skips an effect whose dep did not change,
  // so the one case the re-read exists for — the write returns 200 and the flag
  // is STILL off, which `Config::get_param` produces whenever an env var shadows
  // the config file — left the toggle stuck ON with no error. Clearing the
  // overlay instead means the daemon's answer is what renders, equal or not.
  const [pending, setPending] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const on = pending ?? gate.enabled;

  const save = async (v: boolean) => {
    setPending(v);
    setError(null);
    try {
      await api.upsertConfig(gate.config_key, v);
    } catch (err) {
      // A failed write is never shown as a success.
      setPending(null);
      setError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    try {
      await onFlipped();
    } catch (err) {
      // The write LANDED; only the read-back failed. Saying "Couldn't save"
      // here would be the same lie in the other direction, so the switch drops
      // back to the last value the daemon actually confirmed and says why.
      setError(`Saved, but could not re-read it: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setPending(null);
    }
  };

  return (
    <div data-testid="agent-gate">
      <Section
        title="Enabled"
        sub="One key, written here and under Settings → Features — flipping it in either place is the same flag, and the daemon picks it up at its next tick."
      >
        <Row label={`Enable ${agentName}`} hint={gateRowHint(gate)}>
          <Toggle on={on} onChange={v => { void save(v); }} />
          {error && (
            <div style={{ marginTop: 8, fontSize: 12, color: colors.danger }}>{error}</div>
          )}
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

  // The work review is a snapshot taken on open, so a pass run from this page
  // would otherwise be absent from the very list that answers "did it run?".
  const refreshWork = useCallback(async () => {
    const next = await fetchAgentWork(id);
    setWork(next);
    setWorkError(null);
  }, [id]);

  // Validated, never cast: an older daemon serialises no gate at all, and a
  // missing gate must read as "no switch known" rather than as a switch that is
  // off — offering to flip a key that daemon does not read would be a lie.
  const gate = readAgentGate(detail);

  // Same reasoning as the gate, and the same shape of bug if it were skipped: a
  // daemon that serialises neither field must leave both controls DISABLED with
  // a reason, never enabled on a guess that would fail when pressed.
  const ask = readAgentCapability(detail, 'ask');
  const runNow = readAgentCapability(detail, 'run_now');

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

      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 16 }}>
        <AgentPortrait agentId={id} size={56} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <H1 sub={detail.kind === 'worker' ? detail.what_it_does : detail.role}>
            {detail.display_name}
          </H1>
        </div>
      </div>

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

      {gate && (
        <AgentEnableRow
          gate={gate}
          agentName={detail.display_name}
          onFlipped={reloadAfterFlip}
        />
      )}

      {/* First, above everything else on the page: the user asked to be able to
          ask an agent a question before anything else about it. */}
      <AskAgentBlock agentId={id} agentName={detail.display_name} ask={ask} />

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
                <div style={{ fontSize: 12, color: colors.textMuted, marginBottom: 10 }}>
                  Current: {grantsSummary(detail)}
                </div>
                <div
                  data-testid="grants-not-enforced"
                  style={{ fontSize: 12, color: colors.warning, lineHeight: 1.5 }}
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
        {workError && <div style={{ fontSize: 12, color: colors.danger }}>{workError}</div>}
        {!workError && !work && <div style={{ fontSize: 12, color: colors.textDim }}>Loading work…</div>}
        {work && (
          <WorkReviewBlock
            work={work}
            agentId={id}
            runNow={runNow}
            onRan={refreshWork}
          />
        )}
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
        onDetailReloaded={d => setDetail(d)}
        onRosterStale={loadRoster}
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
      <H1 sub="Workers that run themselves, the dispatch roster goals go to, and capabilities with their declared secrets — three populations, kept honest. Flag-gated workers are listed here whether or not they are switched on, each with its own switch.">
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
          The roster the daemon returned has no agent named {unknownFocus}. Flag-gated workers
          are listed here even while their flag is off, so being switched off is no longer an
          explanation — a daemon that answered with a partial roster still would be.
        </div>
      )}

      <Section
        title="Workers"
        sub="Background workers run themselves and are not dispatchable. A worker whose flag is off is listed here too, marked off — its switch is on its own page."
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

/**
 * The on/off chip is why a gated worker is LISTED rather than hidden: the list
 * itself now answers "is this thing switched on?", which is the question that
 * previously sent the user hunting through five panes for a control they could
 * not find because the row it belonged to was filtered away.
 */
function GateChip({ id, gate }: { id: string; gate: AgentGate }) {
  const { colors } = useTheme();
  return (
    <span
      data-testid={`gate-chip-${id}`}
      style={{
        fontSize: 10, fontWeight: 600, padding: '1px 7px', borderRadius: 999,
        border: `1px solid ${gate.enabled ? colors.borderHi : colors.border}`,
        color: gate.enabled ? colors.cyan : colors.textDim,
        fontFamily: font.body,
      }}
    >
      {gate.enabled ? 'on' : 'off'}
    </span>
  );
}

function WorkerRow({ worker, onOpen }: { worker: BackgroundWorker; onOpen: () => void }) {
  const { colors } = useTheme();
  const live = liveStateLabel(worker.live_state);
  const problem = worker.live_state.status === 'unavailable';
  const gate = readAgentGate(worker);
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
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <AgentPortrait agentId={worker.id} size={28} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12, marginBottom: 4 }}>
            <span style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <span style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{worker.display_name}</span>
              {gate && <GateChip id={worker.id} gate={gate} />}
            </span>
            <StatusText text={live.text} tone={live.tone} />
          </div>
          <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.45 }}>{worker.what_it_does}</div>
        </div>
      </div>
    </button>
  );
}

function PersonaRow({ persona, onOpen }: { persona: DispatchPersona; onOpen: () => void }) {
  const { colors } = useTheme();
  const avail = availabilityLabel(persona.availability);
  const gate = readAgentGate(persona);
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
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 10 }}>
        <AgentPortrait agentId={persona.key} size={28} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12, marginBottom: 4 }}>
            <span style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <span style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{persona.display_name}</span>
              {gate && <GateChip id={persona.key} gate={gate} />}
            </span>
            <StatusText text={avail.text} tone={avail.tone} />
          </div>
          <div style={{ fontSize: 11, color: colors.textMuted, display: 'flex', flexWrap: 'wrap', gap: '4px 12px' }}>
            <span>{engineLabel(persona.engine)}</span>
            <span>{persona.cost_tier}</span>
          </div>
          <div style={{ fontSize: 11, color: colors.textDim, marginTop: 6 }}>{grantsSummary(persona)}</div>
        </div>
      </div>
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
        {requiredSecretHints(capability.required_secrets).map(hint => (
          <div key={hint} data-testid="required-secret-hint" style={{ marginTop: 2 }}>{hint}</div>
        ))}
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
