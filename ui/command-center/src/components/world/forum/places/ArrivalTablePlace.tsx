/**
 * World W2 — Arrival table: understand and brief.
 *
 * Identifies the configured orchestrator and its four-fact state (configured,
 * reachable, ready, actively used — each with its evidence sentence), what is
 * in flight, what waits for review and recent requests (failed and cancelled
 * shown as such). Primary action: prepare (or continue) a brief. Sending is a
 * separate confirmation inside the composer that calls the host's
 * `onRequestSend`; nothing here sends on selection.
 */
import { useRef, useState } from 'react';
import { Button } from '../../../common/Button';
import { truthFromHenryStatus } from '../data/identity';
import { placeSchema } from '../data/placeSchema';
import { visibleItems, type DecisionRecord, type GoalRecord, type OrchestratorRecord, type RequestRecord, type Section } from '../data/types';
import { BriefComposer } from './BriefComposer';
import { EvidenceSlot } from './EvidenceSlot';
import { ageLabel, displayName, goalPhaseLabel, requestOutcomeLabel, truthFacts } from './format';
import type { PlaceHostProps } from './integration';
import { LinkResultNote } from './LinkResultNote';
import { PlacePanel } from './PlacePanel';
import { usePlaceFrame } from './usePlaceFrame';

const SCHEMA = placeSchema('commons');
const slot = (id: string) => SCHEMA.evidence.find((s) => s.id === id)!;

export function ArrivalTablePlace(props: PlaceHostProps) {
  const { projectId, selectedAgentId, drafts, onRequestSend, onEvent, onClose, returnFocusTo, agentNames } = props;
  const { colors, styles, compact, now, snap, openLink, linkResult, stateLine } = usePlaceFrame(props, 'commons', projectId);
  const [composerOpen, setComposerOpen] = useState(false);
  const primaryRef = useRef<HTMLButtonElement>(null);

  const s = snap.snapshot?.sections ?? {};
  const orchestratorSection = s.orchestrator as Section<OrchestratorRecord> | undefined;
  const requests = s.requests as Section<RequestRecord> | undefined;
  const goals = s.active_goals as Section<GoalRecord> | undefined;
  const decisions = s.decisions as Section<DecisionRecord> | undefined;

  const orch = orchestratorSection ? visibleItems(orchestratorSection, now) : { items: [], stale: false, truncated: false };
  const record = orch.items[0] ?? null;
  const truth = record?.truth ?? truthFromHenryStatus(null);
  const hostName = agentNames?.henry ?? record?.name ?? 'Your orchestrator';
  const contextName = selectedAgentId === 'henry' ? hostName : displayName(selectedAgentId, agentNames);
  const sessions = record?.sessions ?? [];
  const openDecisions = decisions ? visibleItems(decisions, now).items : [];

  const restored = drafts.restore('commons', projectId, selectedAgentId);
  const waitingDraft = !composerOpen && !!restored && !restored.sent && restored.brief.trim().length > 0;
  const placeEmpty = !!snap.snapshot && [requests, goals, decisions].every((x) => x?.state === 'empty');

  const facts = (
    <dl style={styles.facts} data-orchestrator-facts={orch.stale ? 'stale' : record ? 'read' : 'not_read'} aria-label={`${hostName}: current state`}>
      {truthFacts(truth).map((f) => (
        <div key={f.id} style={styles.fact} data-fact={f.id} data-value={f.raw}>
          <dt style={styles.label}>{f.label}</dt>
          <dd style={{ ...styles.strong, margin: 0 }}>{f.value}{orch.stale && record ? ' (stale)' : ''}</dd>
          <dd style={{ ...styles.optionMeta, margin: 0 }}>{f.evidence}</dd>
        </div>
      ))}
    </dl>
  );

  return (
    <PlacePanel
      schema={SCHEMA}
      hostName={hostName}
      hostNote="the orchestrator; selecting another agent adds context, it does not assign them"
      stateLine={stateLine([orchestratorSection, requests, goals, decisions])}
      headerExtra={facts}
      primary={{
        id: SCHEMA.primaryAction.id,
        label: SCHEMA.primaryAction.label,
        description: waitingDraft ? `${SCHEMA.primaryAction.description} A draft is waiting from last time.` : SCHEMA.primaryAction.description,
        expanded: composerOpen,
        ref: primaryRef,
        onActivate: () => setComposerOpen(true),
      }}
      secondary={[
        { id: 'sit_together', label: 'Sit together', description: SCHEMA.secondaryActions[0].description, onActivate: () => onEvent?.({ type: 'local_action', placeId: 'commons', action: 'sit_together', agentWorldId: 'henry' }) },
        {
          id: 'review_decision', label: openDecisions.length ? `Review a decision (${openDecisions.length}${decisions?.state === 'ready' && decisions.truncated ? '+' : ''})` : 'Review a decision',
          description: SCHEMA.secondaryActions[2].description,
          onActivate: () => { openLink({ to: 'decision_inbox', decisionId: openDecisions[0]?.id ?? null }); },
        },
        {
          id: 'resume_conversation', label: 'Resume conversation', description: SCHEMA.secondaryActions[3].description,
          disabled: sessions.length === 0, disabledReason: 'no conversation is open on the daemon yet.',
          onActivate: () => { if (sessions[0]) openLink(sessions[0].link); },
        },
        { id: 'open_profile', label: 'Open profile', description: 'Shows the shared agent profile: what it can do, what was reported, and the evidence.', onActivate: () => onEvent?.({ type: 'local_action', placeId: 'commons', action: 'open_profile', agentWorldId: selectedAgentId }) },
        { id: 'explore_places', label: 'Explore places', description: 'Opens the Places list. Every place works without walking.', onActivate: () => onEvent?.({ type: 'local_action', placeId: 'commons', action: 'explore_places' }) },
      ]}
      placeEmpty={placeEmpty}
      compact={compact}
      colors={colors}
      styles={styles}
      onClose={onClose}
      onEvent={onEvent}
      returnFocusTo={returnFocusTo}
    >
      <LinkResultNote linkResult={linkResult} styles={styles} />
      <BriefComposer
        open={composerOpen}
        onClose={() => setComposerOpen(false)}
        triggerRef={primaryRef}
        placeId="commons"
        projectId={projectId}
        agentWorldId={selectedAgentId}
        agentName={contextName}
        heading={`Brief for ${hostName}${selectedAgentId !== 'henry' ? ` · context: ${contextName}` : ''}`}
        intents={['job', 'query']}
        drafts={drafts}
        onRequestSend={onRequestSend}
        onOpenLink={openLink}
        onEvent={onEvent}
        now={now}
        colors={colors}
        styles={styles}
      />
      <div style={styles.body}>
        <div style={styles.column}>
          <EvidenceSlot
            slot={slot('orchestrator')} section={orchestratorSection} loading={snap.loading} now={now} styles={styles} colors={colors} onRetry={snap.retry}
            renderItems={(items) => {
              const r = items[0];
              if (!r) return null;
              return (
                <>
                  <p style={styles.para}>{r.name} reports “{r.currentState}”{r.currentTool ? `, using ${r.currentTool}` : ''} · {r.tasksInFlight} task(s) in flight.</p>
                  {r.sessions.length > 0
                    ? <div style={styles.actions} role="group" aria-label="Continue with">
                        {r.sessions.map((sess) => (
                          <Button key={sess.id} colors={colors} flashSuccess={false} data-record={sess.id} onClick={() => { openLink(sess.link); }}>Continue with “{sess.name}”</Button>
                        ))}
                        {r.sessionsTruncated && <span style={styles.note}>More conversations exist in History.</span>}
                      </div>
                    : <p style={styles.note}>No open conversation to continue.</p>}
                </>
              );
            }}
          />
          <EvidenceSlot
            slot={slot('requests')} section={requests} loading={snap.loading} now={now} styles={styles} colors={colors} collapsible={compact} onRetry={snap.retry}
            renderItems={(items) => (
              <ul style={styles.list}>
                {items.map((r) => (
                  <li key={r.id} style={styles.row} data-record={r.id} data-outcome={r.outcome}>
                    <span style={styles.para}>{r.description}</span>
                    <span style={styles.optionMeta}>
                      <span style={styles.badge(r.outcome === 'failed' ? 'danger' : r.outcome === 'cancelled' ? 'warn' : r.outcome === 'completed' ? 'ok' : 'neutral')}>{requestOutcomeLabel(r.outcome, r.status)}</span>
                      {r.toolUsed ? ` · ${r.toolUsed}` : ''}{r.completedAt ? ` · ${ageLabel(r.completedAt, now)}` : ''}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          />
        </div>
        <div style={styles.column}>
          <EvidenceSlot
            slot={slot('active_goals')} section={goals} loading={snap.loading} now={now} styles={styles} colors={colors} collapsible={compact} moreWhere="Projects" onRetry={snap.retry}
            renderItems={(items) => (
              <>
                <p style={styles.note}>All goals that are ready or executing — not only the orchestrator’s.</p>
                <ul style={styles.list}>
                  {items.map((g) => (
                    <li key={g.id} style={styles.row} data-record={g.id} data-phase={g.phase}>
                      <span style={styles.para}>{g.title}</span>
                      <span style={styles.optionMeta}>{goalPhaseLabel(g.phase, g.state)}{g.assignedTo ? ` · owned by ${g.assignedTo}` : ''} · updated {ageLabel(g.updatedAt, now)}</span>
                      <span><Button colors={colors} variant="bare" flashSuccess={false} onClick={() => { openLink(g.link); }}>Open goal</Button></span>
                    </li>
                  ))}
                </ul>
              </>
            )}
          />
          <EvidenceSlot
            slot={slot('decisions')} section={decisions} loading={snap.loading} now={now} styles={styles} colors={colors} collapsible={compact} moreWhere="the Decision Inbox" onRetry={snap.retry}
            renderItems={(items) => (
              <ul style={styles.list}>
                {items.map((d) => (
                  <li key={d.id} style={styles.row} data-record={d.id}>
                    <span style={styles.para}>{d.headline}</span>
                    <span style={styles.optionMeta}>{d.tier >= 2 ? 'Needs you' : 'Policy may answer'}{d.goalTitle ? ` · ${d.goalTitle}` : ''} · {ageLabel(d.createdAt, now)}</span>
                    <span><Button colors={colors} variant="bare" flashSuccess={false} onClick={() => { openLink(d.link); }}>Review in Decision Inbox</Button></span>
                  </li>
                ))}
              </ul>
            )}
          />
        </div>
      </div>
    </PlacePanel>
  );
}
