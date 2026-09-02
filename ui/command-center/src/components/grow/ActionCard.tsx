/**
 * One action, on the board or on the archived shelf.
 *
 * Split out of GrowView.tsx (R9), unchanged. Its own file for the reason it was
 * already its own component: the archived list renders the same card read-only,
 * and the copy-confirmation flag belongs to a card rather than to an index into
 * a list that reorders on every review.
 */

import { useCallback, useState } from 'react';
import type { CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { GLOSSARY } from '../../lib/vocabulary';
import { Button } from '../common/Button';
import type { Project } from '../projects/types';
import { codingAgentDirective } from './codingAgentDirective';
import { CODING_AGENTS, codingAgentById, codingAgentSelectLabel } from './codingAgents';
import { growSmall } from './growStyles';
import { ARCHIVABLE, categoryColor, verdictMeta, type ActionLane } from './growthWindows';
import type { GrowthAction } from './growTypes';
import { ActionVerify } from './ActionVerify';
import { TrackingRail } from './TrackingRail';

/**
 * One action, on the board or on the archived shelf.
 *
 * Its own top-level component rather than a block inside `GrowActions`' map
 * because the archived list renders the same card read-only, and because the
 * copy-confirmation flag belongs to a card rather than to an index into a list
 * that reorders on every review.
 */
export function ActionCard({
  project, action, colors, lane, onChanged, showCategory = true,
}: {
  project: Project;
  action: GrowthAction;
  colors: ThemeColors;
  /** Which list this card is in — it decides which exits the card offers.
   *
   *  `actions`  work still asking for a decision. Dismiss is offered whatever
   *             the status, because this is the list the user is trying to
   *             shorten and a row here with no control is the defect.
   *  `tracking` work that shipped, shown under the board's "Completed"
   *             heading. Archive is the exit: it files the card away and
   *             KEEPS measuring it, which is what filing away in-flight work
   *             has to mean. Reopen is also offered, but only while nothing
   *             has judged it yet — the server refuses it once outcomes
   *             exist, because reopening clears the pivot those verdicts were
   *             measured from. Dismiss is not offered — it would drop a live
   *             experiment into the refused pile.
   *  `shelf`    archived or dismissed. A record: no controls at all. */
  lane: ActionLane;
  /** Refetch the board. Archiving moves a card between two lists, so the
   *  parent has to re-read rather than this card patching itself. */
  onChanged: () => void;
  /** False inside a category tab — the tab already names the category, and a
   *  chip that repeats it is how the old long tagged list read. */
  showCategory?: boolean;
}) {
  const readOnly = lane === 'shelf';
  const [copied, setCopied] = useState(false);
  const [moving, setMoving] = useState<string | null>(null);
  const [moveError, setMoveError] = useState<string | null>(null);
  const [agentId, setAgentId] = useState<string>(CODING_AGENTS[0].id);
  const [sending, setSending] = useState(false);
  const setPendingTerminalLaunch = useCommandCenter((s) => s.setPendingTerminalLaunch);

  const directive = codingAgentDirective({
    projectName: project.name,
    projectRoot: project.rootPath,
    action,
  });

  const copyArtifact = useCallback((text: string) => {
    navigator.clipboard?.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    });
  }, []);

  const identity = action.identity ?? null;
  const actionId = identity?.id ?? null;

  /** One lifecycle route, two exits, and they are not interchangeable.
   *
   *  Archiving RELEASES an action's text for re-proposal (`board` excludes
   *  archived rows and `restates` is checked against `board`), so it is the
   *  wrong exit for advice the user is done with — and the server refuses it
   *  outright on a `suggested` row for that reason. Dismissal keeps the text on
   *  the generator's board where it can never be proposed again, which is why
   *  it is the exit offered on every card in the Actions list whatever its
   *  status. Without it nothing the user could press ever shortened the panel,
   *  so it could only grow. */
  const move = useCallback((status: string) => {
    if (!actionId) return;
    setMoving(status);
    setMoveError(null);
    apiFetch(
      `/api/projects/${encodeURIComponent(project.id)}/growth-actions/`
      + `${encodeURIComponent(actionId)}/status`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ status }),
      },
    )
      .then(() => onChanged())
      // A refused move says why. The server's refusals are written to be read
      // ("Nothing has happened to this action yet…"), and swallowing one would
      // look like a dead button.
      .catch((e) => setMoveError(e instanceof Error ? e.message : String(e)))
      .finally(() => setMoving(null));
  }, [project.id, actionId, onChanged]);

  /** Sends a Completed card back to Actions with no verdict on record.
   *
   *  A separate route rather than `move('suggested')` because reopening isn't
   *  a status flip: the server has to clear `verified_at`/`verified_by` (and
   *  the commit receipt), since those are the pivot every measurement window
   *  is read from. It reuses `move`'s error pattern — a refused reopen says
   *  why, the same way a refused archive does — and the caller (`canReopen`
   *  below) keeps it off any card whose outcomes already rest on that pivot,
   *  which is also the server's own 409 guard. */
  const reopen = useCallback(() => {
    if (!actionId) return;
    setMoving('reopened');
    setMoveError(null);
    apiFetch(
      `/api/projects/${encodeURIComponent(project.id)}/growth-actions/`
      + `${encodeURIComponent(actionId)}/reopen`,
      { method: 'POST' },
    )
      .then(() => onChanged())
      .catch((e) => setMoveError(e instanceof Error ? e.message : String(e)))
      .finally(() => setMoving(null));
  }, [project.id, actionId, onChanged]);

  const sendToAgent = useCallback(() => {
    const agent = codingAgentById(agentId);
    if (!agent || !project.rootPath) return;
    setSending(true);
    const display = agent.command.split(' ')[0] || agent.label;
    // Queue the launch before switching workspaces: if Build mounts in the
    // same tick, it must already see the pending payload.
    setPendingTerminalLaunch({
      rootPath: project.rootPath,
      label: `${project.slug} · ${display} · grow`,
      command: agent.command,
      followUpInput: directive,
      growthAction: actionId ? { projectId: project.id, actionId } : undefined,
    });
    const opened = navigateToTool('build');
    if (!opened) {
      setMoveError('Open the Build workspace to send this to a coding agent.');
    }
    setSending(false);
  }, [agentId, project.rootPath, project.slug, project.id, directive, actionId, setPendingTerminalLaunch]);

  const tint = categoryColor(action.category, colors);
  const transfer = action.transfer ?? null;
  const canArchive = !readOnly && !!identity && ARCHIVABLE.includes(identity.status);
  /** Completed only, and only before anything has judged it. Once an outcome
   *  exists it was measured from this action's pivot, and reopening would
   *  clear that pivot out from under the verdict — the server refuses the
   *  route for the same reason (409, growth_actions.rs), so a card with
   *  outcomes offers Archive as its only exit rather than a button that would
   *  just come back with an error every time. */
  const canReopen = lane === 'tracking' && !!identity && identity.outcomes.length === 0;
  /** Keyed on the DURABLE ROW, not on a status allowlist and not on the prose
   *  cache.
   *
   *  This was `identity.status === 'suggested'`, which is a claim about
   *  lifecycle where the user's need is about the list: they can see the row,
   *  they have already done it (or never will), and they want it gone. On this
   *  project four actions have been on the board since 2026-08-14 with no entry
   *  left in the prose cache; every control the panel offers hangs off the
   *  identity, so the rule now is simply "the board can reach this row" — which
   *  is true for every card the board renders, because `render_board` builds
   *  the list FROM the rows. `done` in particular had no dismissal at all, and
   *  its only other exit — Archive — releases the text for re-proposal, so
   *  filing away stale advice handed the identical advice back on the next
   *  review. That is exactly what happened here: the 2026-08-19 review restated
   *  the 2026-08-14 funnel action. */
  const canDismiss = lane === 'actions' && !!actionId;

  // `smallButton` survives only for the coding-agent <select>, which borrows
  // the same chrome and is not a button; the buttons take the custom
  // properties so `.pa-btn`'s hover, press and disabled rules apply.
  const smallButton: CSSProperties = {
    background: colors.surface, border: `1px solid ${colors.border}`,
    borderRadius: radius.sm, padding: '3px 10px', cursor: 'pointer',
    color: colors.text, fontFamily: font.body, fontSize: textSize.micro,
  };
  const smallBtn = growSmall(colors);

  return (
    <div style={{
      background: colors.surface, border: `1px solid ${colors.border}`,
      borderRadius: radius.lg, padding: 14,
      // Filed work reads as a record, not as something still asking to be done.
      opacity: readOnly ? 0.75 : 1,
    }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 6 }}>
        {showCategory && (
          <span style={{
            fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
            textTransform: 'uppercase', color: tint,
            border: `1px solid ${tint}`,
            borderRadius: radius.pill, padding: '1px 7px', flexShrink: 0,
          }}>{action.category}</span>
        )}
        <span style={{ fontFamily: font.display, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>
          {action.title}
        </span>
        <div style={{ flex: 1 }} />
        {/* Only when the prose cache still holds both. "medium impact · medium
            confidence" invented for a card whose cache entry was pruned is the
            same fabrication the backend refuses when it declines to default a
            target. */}
        {action.impact && action.confidence && (
          // Two words the card invents and never defines. The gloss says what
          // each measures AND that both are the model's own estimate rather
          // than something measured — which is the part a reader would
          // otherwise have to assume either way.
          <span
            data-testid="action-impact-confidence"
            title={GLOSSARY.impactConfidence}
            style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, flexShrink: 0, cursor: 'help' }}
          >
            {action.impact} impact · {action.confidence} confidence
          </span>
        )}
      </div>

      {/* What this CATEGORY has measurably done elsewhere — derived from
          `growth_action_outcomes` on the user's other active projects, never
          asserted by the model. The provenance disclosure is mandatory: a card
          that appears because something worked elsewhere and will not say where
          is not auditable, and is indistinguishable from the model flattering
          its own suggestion. */}
      {transfer && (
        <div style={{ marginBottom: 6, display: 'flex', flexDirection: 'column', gap: 2 }}>
          <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
            {transfer.helped > 0
              ? `Worked on ${transfer.helped} of ${transfer.projects} other project(s)`
                + ` — on projects like this one, ${transfer.segmentHelped} of`
                + ` ${transfer.segmentProjects} (${transfer.segmentLabel})`
              : transfer.hindered > 0
                ? `Hindered on ${transfer.hindered} of ${transfer.projects} other project(s)`
                : `Tried on ${transfer.projects} other project(s), with no detectable change`}
          </span>
          {transfer.examples.length > 0 && (
            <details>
              <summary style={{
                fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
                textTransform: 'uppercase', color: colors.textDim, cursor: 'pointer',
              }}>Where that comes from</summary>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2, marginTop: 4 }}>
                {transfer.examples.map((ex, xi) => (
                  <div key={`${ex.projectName}-${xi}`} style={{ fontSize: textSize.micro, color: colors.textDim }}>
                    &ldquo;{ex.title}&rdquo; on {ex.projectName} —{' '}
                    {verdictMeta(ex.verdict, colors).label}
                    {ex.deltaPct !== null && (
                      `, ${ex.deltaPct > 0 ? '+' : ''}${(ex.deltaPct * 100).toFixed(0)}%`
                    )}
                  </div>
                ))}
              </div>
            </details>
          )}
        </div>
      )}

      {/* Evidence first: the number is what makes this checkable. Rendered only
          when there is one — an empty rail beside a card whose prose the cache
          no longer holds reads as a missing figure rather than as no figure. */}
      {action.evidence && (
        <div style={{
          fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono,
          borderLeft: `2px solid ${colors.border}`, paddingLeft: 8, marginBottom: 6,
        }}>{action.evidence}</div>
      )}
      <div style={{ fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.5 }}>
        {action.recommendation}
      </div>

      {/* Ordered steps: an action nobody knows how to start is an observation
          wearing an action's clothes. */}
      {action.steps?.length > 0 && (
        <ol style={{ margin: '8px 0 0', paddingLeft: 18, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.6 }}>
          {action.steps.map((step, si) => <li key={si}>{step}</li>)}
        </ol>
      )}

      {/* Always a coding-agent prompt, even when the generator stored a bare
          post. Copying the raw artifact is how SEO work landed in chat as a
          blog post with no path and no instruction. */}
      {lane === 'actions' && (
        <div style={{ marginTop: 10 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4, flexWrap: 'wrap' }}>
            <span style={{ fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em', textTransform: 'uppercase', color: colors.textDim }}>
              Prompt for your coding agent
            </span>
            <div style={{ flex: 1 }} />
            {/* Its own "Copied ✓" already confirms the copy, so the primitive's
                tick would say the same thing twice. */}
            <Button
              colors={colors}
              onClick={() => copyArtifact(directive)}
              flashSuccess={false}
              style={smallBtn}
            >{copied ? 'Copied ✓' : 'Copy'}</Button>
            <select
              aria-label="Coding agent"
              value={agentId}
              onChange={(e) => setAgentId(e.target.value)}
              style={{
                ...smallButton, cursor: 'pointer',
                opacity: project.rootPath ? 1 : 0.5,
              }}
            >
              {CODING_AGENTS.map((a) => (
                <option key={a.id} value={a.id} title={a.tooltip}>
                  {codingAgentSelectLabel(a)}
                </option>
              ))}
            </select>
            <Button
              colors={colors}
              onClick={sendToAgent}
              disabled={sending || !project.rootPath}
              title={!project.rootPath
                ? 'Add a root path to this project to launch a coding agent here.'
                : `Open ${codingAgentById(agentId)?.label ?? 'the agent'} in Build with this prompt`}
              style={smallBtn}
            >{sending ? 'Sending…' : 'Send'}</Button>
          </div>
          <pre style={{
            margin: 0, background: colors.bgDeeper, border: `1px solid ${colors.border}`,
            borderRadius: radius.md, padding: 10, fontSize: textSize.micro, fontFamily: font.mono,
            color: colors.textMuted, whiteSpace: 'pre-wrap', wordBreak: 'break-word',
            maxHeight: 200, overflowY: 'auto',
          }}>{directive}</pre>
        </div>
      )}

      {/* OUTSIDE the artifact block on purpose. Verification applies to every
          action including artifactKind "none" — that is the `self` fallback row
          of the proposal's table (proposal:105) — and putting this beside Copy
          would hide it for exactly the actions with no deliverable to copy. */}
      <ActionVerify
        key={actionId ?? action.title}
        projectId={project.id}
        action={action}
        colors={colors}
        onChanged={onChanged}
        readOnly={readOnly}
      />

      {/* Only on the Tracking lane. On the Actions lane there is nothing to
          measure yet, and on the shelf the card is a record. */}
      {lane === 'tracking' && identity && (
        <TrackingRail identity={identity} colors={colors} />
      )}

      {canArchive && (
        <div style={{ marginTop: 8, display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          {/* Not deletion, and the wording must not promise permanence: an
              archived action keeps being measured while it still owes a window,
              keeps feeding the agent's learning, and releases its text back for
              re-proposal. */}
          <Button
            colors={colors}
            onClick={() => move('archived')}
            disabled={!!moving}
            pending={moving === 'archived'}
            style={smallBtn}
          >{moving === 'archived' ? 'Filing…' : 'Archive'}</Button>
          {canReopen && (
            <Button
              colors={colors}
              onClick={reopen}
              disabled={!!moving}
              pending={moving === 'reopened'}
              style={smallBtn}
            >{moving === 'reopened' ? 'Reopening…' : 'Reopen'}</Button>
          )}
          <span style={{ fontSize: 10, color: colors.textDim }}>
            Files it away. It keeps being measured and keeps teaching the agent.
          </span>
        </div>
      )}
      {canDismiss && (
        <div style={{ marginTop: 8, display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          <Button
            colors={colors}
            onClick={() => move('dismissed')}
            disabled={!!moving}
            pending={moving === 'dismissed'}
            style={smallBtn}
          >{moving === 'dismissed' ? 'Dismissing…' : 'Not interested'}</Button>
          {/* The distinction matters and is the whole reason dismissal is not
              archiving: a dismissed action stays ON the board, so the generator
              still sees it and cannot propose it again. Archiving releases the
              text. */}
          <span style={{ fontSize: 10, color: colors.textDim }}>
            Takes it off the list. The agent keeps it in view, so it will not suggest it again.
          </span>
        </div>
      )}
      {moveError && (
        <div style={{ marginTop: 6, fontSize: textSize.micro, color: colors.danger }}>{moveError}</div>
      )}
    </div>
  );
}
