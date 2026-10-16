import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import {
  api,
  type GuardFinding,
  type GuardStatus,
  type GuardSummary,
  type GuardVerifyResult,
} from '../../lib/api';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { plainText } from '../../lib/plainText';
import { Panel } from './Panel';
import { DetailModal } from '../common/DetailModal';
import type { Project } from './types';
import { guardFixDirective } from '../grow/guardFixDirective';
import { CODING_AGENTS, codingAgentById, codingAgentSelectLabel } from '../grow/codingAgents';
import { rememberedAgent } from '../grow/ActionCard';

// The Guard's findings for one project, on GET /api/projects/{id}/guard.
// The Guard never edits code: a finding goes to a coding agent, the Guard
// checks the commit, and a rescan confirms. With no active findings the panel
// says WHY — off / never scanned / sweep did not finish / scanned clean —
// because silence used to read as "secure" whether or not the Guard had run.

export const DISMISS_MAX = 280;
const SHOWN = 8;

function formatDate(iso: string | null | undefined): string {
  if (!iso) return '';
  try {
    return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  } catch {
    return iso;
  }
}

const clampLines = (n: number): CSSProperties => ({
  display: '-webkit-box', WebkitLineClamp: n, WebkitBoxOrient: 'vertical', overflow: 'hidden',
});

const statusOf = (f: GuardFinding): GuardStatus => f.status ?? 'open';
export const isClosed = (f: GuardFinding) => {
  const s = statusOf(f);
  return s === 'fixed' || s === 'dismissed';
};

/** Plain-language chip for a finding's state. */
export function statusLabel(f: GuardFinding, agentLabel?: (id: string) => string): string {
  switch (statusOf(f)) {
    case 'sent': {
      const who = f.sent_agent ? (agentLabel ? agentLabel(f.sent_agent) : f.sent_agent) : 'your coding agent';
      return `Sent to ${who}`;
    }
    case 'fix_committed': return 'Fix committed (waiting for rescan)';
    case 'not_seen': return 'Not seen in latest scan (unconfirmed)';
    case 'reopened': return 'Reopened';
    case 'fixed': return 'Fixed';
    case 'dismissed': return 'Dismissed';
    default: return 'Open';
  }
}

const agentName = (id: string) => codingAgentById(id)?.label ?? id;

export function GuardPanel({ project, compact = false }: { project: Project; compact?: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const { colors } = useTheme();
  const setPendingTerminalLaunch = useCommandCenter(s => s.setPendingTerminalLaunch);
  const [guard, setGuard] = useState<GuardSummary | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [showClosed, setShowClosed] = useState(false);
  const [showAll, setShowAll] = useState(false);
  const [agentId, setAgentIdState] = useState<string>(() => rememberedAgent(project.id));
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<Record<string, string>>({});
  const [verifyResult, setVerifyResult] = useState<Record<string, GuardVerifyResult>>({});
  const [dismissing, setDismissing] = useState<string | null>(null);
  const [reason, setReason] = useState('');
  const [rescanNote, setRescanNote] = useState<string | null>(null);

  const setAgentId = (id: string) => {
    setAgentIdState(id);
    try { localStorage.setItem(`grow.sendAgent.${project.id}`, id); } catch { /* storage can be unavailable */ }
  };

  const load = useCallback(async () => {
    try {
      const g = await api.getGuard(project.id);
      setGuard(g);
      setLoadFailed(false);
    } catch {
      setLoadFailed(true);
    }
  }, [project.id]);

  useEffect(() => { void load(); }, [load]);

  const say = (id: string, text: string | null) =>
    setNotice(n => {
      const next = { ...n };
      if (text) next[id] = text; else delete next[id];
      return next;
    });

  const fixWithAgent = async (f: GuardFinding) => {
    const agent = codingAgentById(agentId);
    if (!agent || !project.rootPath) return;
    say(f.id, null);
    setBusy(`fix:${f.id}`);
    // Queue the launch before switching workspaces so Build sees it on mount.
    setPendingTerminalLaunch({
      rootPath: project.rootPath,
      label: `${project.slug} · ${agent.command.split(' ')[0] || agent.label} · guard`,
      command: agent.command,
      followUpInput: guardFixDirective({
        projectName: project.name,
        projectRoot: project.rootPath,
        projectSlug: project.slug,
        finding: f,
      }),
    });
    const opened = navigateToTool('build');
    if (!opened) {
      say(f.id, 'Open the Build workspace to send this to a coding agent.');
      setBusy(null);
      return;
    }
    try {
      await api.markGuardFindingSent(project.id, f.id, agent.id);
    } catch (e) {
      say(f.id, `Sent to the agent, but the Guard could not record it: ${e instanceof Error ? e.message : String(e)}`);
    }
    await load();
    setBusy(null);
  };

  const verify = async (f: GuardFinding) => {
    setBusy(`verify:${f.id}`);
    say(f.id, null);
    try {
      const r = await api.verifyGuardFinding(project.id, f.id);
      setVerifyResult(v => ({ ...v, [f.id]: r }));
    } catch (e) {
      say(f.id, `Could not check this fix: ${e instanceof Error ? e.message : String(e)}`);
    }
    await load();
    setBusy(null);
  };

  const reasonTrim = reason.trim();
  const reasonError = reasonTrim.length === 0
    ? 'Give a reason (1–280 characters).'
    : reasonTrim.length > DISMISS_MAX
      ? `Reason is ${reasonTrim.length} characters; the limit is ${DISMISS_MAX}.`
      : null;

  const dismiss = async (f: GuardFinding) => {
    if (reasonError) return;
    setBusy(`dismiss:${f.id}`);
    say(f.id, null);
    try {
      await api.setGuardFindingStatus(project.id, f.id, { status: 'dismissed', reason: reasonTrim });
      setDismissing(null);
      setReason('');
    } catch (e) {
      say(f.id, `Could not dismiss: ${e instanceof Error ? e.message : String(e)}`);
    }
    await load();
    setBusy(null);
  };

  const reopen = async (f: GuardFinding) => {
    setBusy(`reopen:${f.id}`);
    say(f.id, null);
    try {
      await api.setGuardFindingStatus(project.id, f.id, { status: 'open' });
    } catch (e) {
      say(f.id, `Could not reopen: ${e instanceof Error ? e.message : String(e)}`);
    }
    await load();
    setBusy(null);
  };

  const rescan = async () => {
    setBusy('rescan');
    setRescanNote(null);
    try {
      const r = await api.requestGuardRescan(project.id);
      setRescanNote(r.queued
        ? 'Rescan queued. The Guard will scan this project shortly.'
        : `Rescan not started: ${r.reason || 'the Guard declined.'}`);
    } catch (e) {
      setRescanNote(`Rescan not started: ${e instanceof Error ? e.message : String(e)}`);
    }
    await load();
    setBusy(null);
  };

  const small: CSSProperties = {
    fontSize: textSize.micro, fontFamily: font.body, padding: `${space.xs}px ${space.md}px`,
  };
  const severityColor = (s: string) =>
    s === 'high' || s === 'critical' ? colors.danger : s === 'medium' ? colors.warning : colors.textDim;

  if (!guard) {
    return (
      <Panel title="Security — from the Guard">
        <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
          {loadFailed ? 'Could not read the Guard’s findings for this project.' : 'Reading the Guard’s findings…'}
        </div>
        {loadFailed && (
          <div style={{ marginTop: space.md }}>
            <Button colors={colors} onClick={() => void load()} style={small}>Try again</Button>
          </div>
        )}
      </Panel>
    );
  }

  const active = guard.findings.filter(f => !isClosed(f));
  const closed = guard.findings.filter(isClosed);
  const { counts } = guard;
  const shown = showAll ? active : active.slice(0, SHOWN);

  const rescanLabel = busy === 'rescan' ? 'Queuing…' : 'Rescan now';
  const rescanButton = (
    <Button colors={colors} onClick={() => void rescan()} disabled={busy === 'rescan' || !guard.enabled} style={small}
      title={guard.enabled ? 'Ask the Guard to scan this project now' : 'The Guard is off'}>
      {rescanLabel}
    </Button>
  );

  const summary = [
    `${counts.active} active`,
    counts.open > 0 ? `${counts.open} open` : null,
    counts.sent > 0 ? `${counts.sent} sent to an agent` : null,
    counts.awaiting_rescan > 0 ? `${counts.awaiting_rescan} awaiting rescan` : null,
    counts.unconfirmed > 0 ? `${counts.unconfirmed} unconfirmed` : null,
    `${counts.fixed} fixed`,
    `${counts.dismissed} dismissed`,
  ].filter(Boolean).join(' · ');

  let emptyText: string | null = null;
  if (active.length === 0) {
    const attemptFailed = Boolean(
      guard.last_attempt && (!guard.last_scan || guard.last_attempt > guard.last_scan),
    );
    emptyText = attemptFailed
      ? `Last sweep did not finish (${formatDate(guard.last_attempt)})${guard.last_scan ? ` — previous clean scan ${formatDate(guard.last_scan)}` : ' — not a clean scan.'}`
      : guard.last_scan
        ? `Scanned ${formatDate(guard.last_scan)} — no open findings.${guard.enabled ? '' : ' The Guard is currently off.'}`
        : !guard.enabled
          ? 'The Guard is off — enable security sweeps in Settings → Agents to scan this project.'
          : 'Never scanned — waiting on the Guard’s first sweep of this project.';
  }

  const chipColor = (f: GuardFinding): string => {
    switch (statusOf(f)) {
      case 'reopened': return colors.danger;
      case 'fix_committed': return colors.success;
      case 'not_seen': return colors.warning;
      case 'fixed': return colors.success;
      case 'sent': return colors.cyan;
      default: return colors.textMuted;
    }
  };

  const renderFinding = (f: GuardFinding) => {
    const closedItem = isClosed(f);
    const res = verifyResult[f.id];
    const st = statusOf(f);
    return (
      <div key={f.id} data-testid="guard-finding" style={{ display: 'flex', gap: space.lg, alignItems: 'baseline' }}>
        <span style={{
          fontSize: 9, fontWeight: 700, letterSpacing: 0.6, flexShrink: 0,
          textTransform: 'uppercase', color: severityColor(f.severity),
          lineHeight: '18px', width: 52,
        }}>
          {f.severity}
        </span>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ fontSize: textSize.caption, color: colors.text, lineHeight: 1.5, ...clampLines(2) }}>
            {plainText(f.title)}
          </div>
          <div style={{ fontSize: 10, color: colors.textDim, marginTop: space.xxs, display: 'flex', gap: space.md, flexWrap: 'wrap', alignItems: 'center' }}>
            <span data-testid="guard-chip" style={{
              fontWeight: 600, color: chipColor(f), border: `1px solid ${colors.border}`,
              borderRadius: radius.xs, padding: `0 ${space.sm}px`,
            }}>{statusLabel(f, agentName)}</span>
            {f.cwe && <span>{f.cwe}</span>}
            {f.location && <span style={{ fontFamily: font.mono }}>{f.location}</span>}
            <span>{formatDate(f.found_at)}</span>
          </div>
          {f.remediation && plainText(f.remediation) !== plainText(f.title) && !closedItem && (
            <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xxs, lineHeight: 1.5, ...clampLines(3) }}>
              {plainText(f.remediation)}
            </div>
          )}
          {st === 'fixed' && f.fix_commit && (
            <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xxs }}>
              Fixed in <span style={{ fontFamily: font.mono }}>{f.fix_commit.slice(0, 7)}</span>
              {f.fix_commit_subject ? ` — ${f.fix_commit_subject}` : ''}
            </div>
          )}
          {st === 'fix_committed' && f.fix_commit && (
            <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xxs }}>
              Commit <span style={{ fontFamily: font.mono }}>{f.fix_commit.slice(0, 7)}</span>
              {f.fix_commit_subject ? ` — ${f.fix_commit_subject}` : ''}. The next scan will confirm it.
            </div>
          )}
          {st === 'dismissed' && f.dismissed_reason && (
            <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xxs }}>
              Reason: {f.dismissed_reason}
            </div>
          )}
          {res && (
            <div data-testid="guard-verify" style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xs, lineHeight: 1.5 }}>
              <div style={{ color: res.verified ? colors.success : colors.text }}>
                {res.verified ? 'Fix found.' : 'No fix found yet.'} {res.detail}
              </div>
              {res.checks.map(c => (
                <div key={c.id}>{c.passed ? 'Yes' : 'No'} — {c.label}: {c.detail}</div>
              ))}
              {res.verified && res.rescan_requested && <div>A rescan was requested to confirm it.</div>}
            </div>
          )}
          {notice[f.id] && (
            <div role="status" style={{ fontSize: textSize.micro, color: colors.warning, marginTop: space.xs }}>{notice[f.id]}</div>
          )}
          <div style={{ display: 'flex', gap: space.sm, marginTop: space.sm, flexWrap: 'wrap' }}>
            {!closedItem && (
              <>
                {f.status !== 'fix_committed' && (
                <Button colors={colors} style={small}
                  disabled={!project.rootPath || busy === `fix:${f.id}`}
                  title={project.rootPath
                    ? `Open ${codingAgentById(agentId)?.label ?? 'the agent'} in Build with this finding`
                    : 'Add a root path to this project to launch a coding agent here.'}
                  onClick={() => void fixWithAgent(f)}>
                  Fix with agent
                </Button>
                )}
                <Button colors={colors} style={small} disabled={busy === `verify:${f.id}`}
                  onClick={() => void verify(f)}>
                  {busy === `verify:${f.id}` ? 'Checking…' : 'Verify fix'}
                </Button>
                <Button colors={colors} style={small} onClick={() => { setDismissing(f.id); setReason(''); }}>
                  Dismiss
                </Button>
              </>
            )}
            {closedItem && (
              <Button colors={colors} style={small} disabled={busy === `reopen:${f.id}`} onClick={() => void reopen(f)}>
                Reopen
              </Button>
            )}
          </div>
          {dismissing === f.id && (
            <div style={{ marginTop: space.sm, display: 'flex', flexDirection: 'column', gap: space.xs }}>
              <input
                aria-label="Reason for dismissing"
                value={reason}
                onChange={e => setReason(e.target.value)}
                placeholder="Why is this not a problem? (1–280 characters)"
                style={{
                  fontSize: textSize.micro, fontFamily: font.body, padding: `${space.xs}px ${space.md}px`,
                  background: colors.fillSubtle, color: colors.text,
                  border: `1px solid ${colors.border}`, borderRadius: radius.xs,
                }}
              />
              {reason.length > 0 && reasonError && (
                <div role="alert" style={{ fontSize: textSize.micro, color: colors.warning }}>{reasonError}</div>
              )}
              <div style={{ display: 'flex', gap: space.sm }}>
                <Button colors={colors} style={small} disabled={Boolean(reasonError) || busy === `dismiss:${f.id}`}
                  onClick={() => void dismiss(f)}>
                  Confirm dismiss
                </Button>
                <Button colors={colors} style={small} onClick={() => { setDismissing(null); setReason(''); }}>
                  Cancel
                </Button>
              </div>
            </div>
          )}
        </div>
      </div>
    );
  };

  const fullPanel = (
    <Panel title="Security — from the Guard" action={rescanButton}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.xl }}>
        <div data-testid="guard-summary" style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
          {summary}
        </div>
        {rescanNote && (
          <div role="status" data-testid="guard-rescan-note" style={{ fontSize: textSize.micro, color: colors.textMuted }}>
            {rescanNote}
          </div>
        )}
        {!guard.enabled && active.length > 0 && (
          <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
            The Guard is currently off. These are from earlier scans.
          </div>
        )}
        {emptyText && (
          <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>{emptyText}</div>
        )}
        {active.length > 0 && (
          <div style={{ display: 'flex', alignItems: 'center', gap: space.md }}>
            <span style={{ fontSize: textSize.micro, color: colors.textDim }}>Send to</span>
            <select
              aria-label="Coding agent"
              value={agentId}
              onChange={e => setAgentId(e.target.value)}
              style={{ ...small, cursor: 'pointer', opacity: project.rootPath ? 1 : 0.5 }}
            >
              {CODING_AGENTS.map(a => (
                <option key={a.id} value={a.id} title={a.tooltip}>{codingAgentSelectLabel(a)}</option>
              ))}
            </select>
          </div>
        )}
        {shown.map(renderFinding)}
        {active.length > SHOWN && (
          <Button colors={colors} style={small} onClick={() => setShowAll(v => !v)}>
            {showAll ? 'Show fewer' : `Show ${active.length - SHOWN} more`}
          </Button>
        )}
        {closed.length > 0 && (
          <div>
            <Button colors={colors} style={small} aria-expanded={showClosed} onClick={() => setShowClosed(v => !v)}>
              {showClosed ? 'Hide' : 'Show'} closed ({counts.fixed} fixed, {counts.dismissed} dismissed)
            </Button>
            {showClosed && (
              <div data-testid="guard-closed" style={{ display: 'flex', flexDirection: 'column', gap: space.xl, marginTop: space.lg }}>
                {closed.map(renderFinding)}
              </div>
            )}
          </div>
        )}
      </div>
    </Panel>
  );
  if (!compact) return fullPanel;
  const urgent = active.filter(f => f.severity === 'high' || f.severity === 'critical').length;
  const severityRank = (f: GuardFinding) => {
    const rank = ['critical', 'high', 'medium', 'low'].indexOf(f.severity);
    return rank < 0 ? 4 : rank;
  };
  const preview = [...active].sort((a, b) => severityRank(a) - severityRank(b)).slice(0, 2);
  return (
    <>
      <Panel title="Security" action={<Button colors={colors} variant="bare" onClick={() => setExpanded(true)}>Review security</Button>}>
        <div style={{ fontSize: textSize.caption, color: urgent ? colors.danger : colors.textMuted }}>
          {counts.active} active findings{urgent > 0 ? ` · ${urgent} high or critical` : ''}
        </div>
        {emptyText && <p style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>{emptyText}</p>}
        {!guard.enabled && active.length > 0 && <p style={{ fontSize: textSize.micro, color: colors.textDim }}>The Guard is off. These findings are from earlier scans.</p>}
        {preview.map(f => <div key={f.id} style={{ marginTop: space.md, fontSize: textSize.caption, ...clampLines(2) }}>
          <span style={{ color: severityColor(f.severity) }}>{f.severity} · </span>{plainText(f.title)}
        </div>)}
        {active.length > 0 && <div style={{ marginTop: space.md, fontSize: textSize.micro, color: colors.textDim }}>
          {guard.last_scan ? `Last scan ${formatDate(guard.last_scan)}` : 'No completed scan recorded'}
          {guard.last_attempt && (!guard.last_scan || guard.last_attempt > guard.last_scan) ? ' · Latest sweep did not finish' : ''}
        </div>}
      </Panel>
      {expanded && <DetailModal title="Project security" width={820} onClose={() => setExpanded(false)}>{fullPanel}</DetailModal>}
    </>
  );
}
