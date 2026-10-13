/**
 * World W2 — Maker workshop: follow work to a result.
 *
 * Shows where each piece of work stands — draft, sent, tracked, needs review,
 * verified — and shows failed and cancelled work as such. A sent request does
 * not put anything on the result shelf: only a goal that is actually complete
 * appears there, and "verified" needs a review decision on record. Primary
 * action: open the real goal behind the selected work. Approval stays in the
 * Decision Inbox and the goal detail.
 */
import { useMemo, useRef, useState } from 'react';
import { Button } from '../../../common/Button';
import { placeSchema } from '../data/placeSchema';
import { recordLinkKey } from '../data/recordLinks';
import { visibleItems, type ActivityRecord, type DecisionRecord, type GoalRecord, type Section, type SkillRecord, type SpendRecord } from '../data/types';
import { BriefComposer } from './BriefComposer';
import { EvidenceSlot } from './EvidenceSlot';
import { ageLabel, displayName, goalPhaseLabel, goalStage, STAGE_LABEL, usd, type WorkStage } from './format';
import type { PlaceHostProps } from './integration';
import { LinkResultNote } from './LinkResultNote';
import { PlacePanel } from './PlacePanel';
import { RecordListbox } from './RecordListbox';
import { usePlaceFrame } from './usePlaceFrame';

const SCHEMA = placeSchema('maker');
const slot = (id: string) => SCHEMA.evidence.find((s) => s.id === id)!;
const action = (id: string) => SCHEMA.secondaryActions.find((a) => a.id === id)!;

const stageTone = (stage: WorkStage) =>
  stage === 'failed' ? 'danger' as const
    : stage === 'cancelled' || stage === 'needs_review' || stage === 'complete_unreviewed' || stage === 'unknown' ? 'warn' as const
      : stage === 'verified' ? 'ok' as const : 'neutral' as const;

export function MakerWorkshopPlace(props: PlaceHostProps) {
  const { projectId, selectedAgentId, drafts, onRequestSend, onEvent, onClose, returnFocusTo, agentNames } = props;
  const { colors, styles, compact, now, snap, openLink, linkResult, stateLine } = usePlaceFrame(props, 'maker', projectId);
  const [picked, setPicked] = useState<string | null>(null);
  const [composerOpen, setComposerOpen] = useState(false);
  const repairRef = useRef<HTMLButtonElement>(null);

  const s = snap.snapshot?.sections ?? {};
  const goals = s.steward_goals as Section<GoalRecord> | undefined;
  const activity = s.steward_activity as Section<ActivityRecord> | undefined;
  const spend = s.steward_spend as Section<SpendRecord> | undefined;
  const decisions = s.decisions as Section<DecisionRecord> | undefined;
  const skills = s.skills as Section<SkillRecord> | undefined;

  const goalItems = goals ? visibleItems(goals, now).items : [];
  const current = goalItems.find((g) => g.phase === 'executing' || g.phase === 'review') ?? goalItems[0] ?? null;
  const selected = goalItems.find((g) => recordLinkKey(g.link) === picked) ?? current;
  const selectedKey = selected ? recordLinkKey(selected.link) : null;
  const results = goalItems.filter((g) => g.phase === 'complete');
  const openDecisions = decisions ? visibleItems(decisions, now).items : [];
  const hostName = displayName('steward', agentNames);
  const placeEmpty = !!snap.snapshot && [goals, activity, decisions, skills].every((x) => x?.state === 'empty');

  // The request trail is local: what you prepared or sent from any place.
  // Re-read whenever the snapshot or composer changes (cheap, local storage).
  const trail = useMemo(() => Object.values(drafts.all().drafts)
    .filter((d) => d.sent || d.brief.trim())
    .sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : -1))
    .slice(0, 6), [drafts, snap.snapshot, composerOpen]);

  const select = (key: string) => {
    setPicked(key);
    onEvent?.({ type: 'record_selected', placeId: 'maker', recordKey: key });
  };

  return (
    <PlacePanel
      schema={SCHEMA}
      hostName={hostName}
      hostNote="works at the bench only when the daemon reports Steward work"
      stateLine={stateLine([goals, activity, spend, decisions, skills])}
      primary={{
        id: SCHEMA.primaryAction.id,
        label: SCHEMA.primaryAction.label,
        description: SCHEMA.primaryAction.description,
        disabled: !selected,
        disabledReason: goals?.state === 'error' && !goalItems.length ? 'the Steward’s work could not be read.' : 'no goal is attributed to the Steward yet. Your sent requests are listed below.',
        onActivate: () => { if (selected) openLink(selected.link); },
      }}
      secondary={[
        { id: 'compare_result', label: action('compare_result').label, description: action('compare_result').description, disabled: !selected, disabledReason: 'choose a goal first.', onActivate: () => { if (selected) openLink(selected.link); } },
        { id: 'review_required', label: openDecisions.length ? `${action('review_required').label} (${openDecisions.length})` : action('review_required').label, description: action('review_required').description, onActivate: () => { openLink({ to: 'decision_inbox', decisionId: openDecisions[0]?.id ?? null }); } },
        { id: 'prepare_repair', label: action('prepare_repair').label, description: action('prepare_repair').description, expanded: composerOpen, ref: repairRef, onActivate: () => setComposerOpen(true) },
        { id: 'inspect_skill', label: action('inspect_skill').label, description: action('inspect_skill').description, onActivate: () => { openLink({ to: 'skills' }); } },
        { id: 'open_build', label: action('open_build').label, description: action('open_build').description, onActivate: () => { openLink({ to: 'build' }); } },
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
        triggerRef={repairRef}
        placeId="maker"
        projectId={projectId}
        agentWorldId={selectedAgentId}
        agentName={selectedAgentId === 'henry' ? hostName : displayName(selectedAgentId, agentNames)}
        heading="Repair or capability brief"
        intents={['job', 'capability']}
        seedEvidence={selected ? [recordLinkKey(selected.link)] : undefined}
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
          <section style={styles.slot} aria-label="Your requests" data-slot="request_trail">
            <p style={styles.slotTitle}>Your requests</p>
            {trail.length === 0
              ? <p style={styles.note}>Nothing prepared or sent from the World yet. Prepare a brief at the Arrival table.</p>
              : (
                <ul style={styles.list}>
                  {trail.map((d) => {
                    const stage: WorkStage = d.sent ? 'sent' : 'draft';
                    return (
                      <li key={`${d.placeId}|${d.projectId ?? '-'}|${d.agentWorldId}`} style={styles.row} data-stage={stage}>
                        <span style={styles.para}>{d.brief.trim() || '(no outcome written)'}</span>
                        <span style={styles.optionMeta}>
                          <span style={styles.badge(stage === 'sent' ? 'ok' : 'neutral')}>{STAGE_LABEL[stage]}</span>
                          {' '}{placeSchema(d.placeId).name}{d.sent ? ` · ${ageLabel(d.sent.at, now)} · ${d.sent.acknowledgement}` : ` · edited ${ageLabel(d.updatedAt, now)}`}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              )}
            <p style={styles.note}>A sent request becomes tracked work only when a goal exists. If the Steward owns it, it appears under Steward goals; other goals are listed at the Arrival table.</p>
          </section>
          <EvidenceSlot
            slot={slot('steward_goals')} section={goals} loading={snap.loading} now={now} styles={styles} colors={colors} moreWhere="Projects" onRetry={snap.retry}
            renderItems={(items) => (
              <RecordListbox
                label="Steward goals"
                items={items}
                keyOf={(g) => recordLinkKey(g.link)}
                selectedKey={selectedKey}
                onSelect={select}
                styles={styles}
                renderOption={(g) => {
                  const stage = goalStage(g);
                  return (
                    <>
                      <span style={styles.para}>{g.title}</span>
                      <span style={styles.optionMeta} data-stage={stage}>
                        <span style={styles.badge(stageTone(stage))}>{STAGE_LABEL[stage]}</span>
                        {' '}{goalPhaseLabel(g.phase, g.state)} · updated {ageLabel(g.updatedAt, now)}
                        {g.reviewDecisions?.status === 'ok' ? ` · ${g.reviewDecisions.count}${g.reviewDecisions.truncated ? '+' : ''} review decision(s)` : g.reviewDecisions?.status === 'unavailable' ? ' · review history unavailable' : ''}
                      </span>
                    </>
                  );
                }}
              />
            )}
          />
          <EvidenceSlot
            slot={slot('steward_activity')} section={activity} loading={snap.loading} now={now} styles={styles} colors={colors} collapsible={compact} onRetry={snap.retry}
            renderItems={(items) => (
              <ul style={styles.list}>
                {items.map((a) => (
                  <li key={a.id} style={styles.row} data-record={a.id}>
                    <span style={styles.para}>{a.title}</span>
                    <span style={styles.optionMeta}>{a.activityKind.replace(/_/g, ' ')} · {ageLabel(a.ts, now)}</span>
                    {a.link && <span><Button colors={colors} variant="bare" flashSuccess={false} onClick={() => { openLink(a.link!); }}>Open</Button></span>}
                  </li>
                ))}
              </ul>
            )}
          />
        </div>
        <div style={styles.column}>
          <section style={styles.slot} aria-label="Result shelf" data-slot="result_shelf" data-results={results.length}>
            <p style={styles.slotTitle}>Result shelf</p>
            {results.length === 0
              ? <p style={styles.note}>No result record yet. A request reaching the server does not put anything on this shelf; only a completed goal does.</p>
              : (
                <ul style={styles.list}>
                  {results.map((g) => {
                    const stage = goalStage(g);
                    return (
                      <li key={g.id} style={styles.row} data-record={g.id} data-stage={stage}>
                        <span style={styles.para}>{g.title}</span>
                        <span style={styles.optionMeta}><span style={styles.badge(stageTone(stage))}>{STAGE_LABEL[stage]}</span> · {ageLabel(g.updatedAt, now)}</span>
                        <span><Button colors={colors} variant="bare" flashSuccess={false} onClick={() => { openLink(g.link); }}>Open the result and its evidence</Button></span>
                      </li>
                    );
                  })}
                </ul>
              )}
          </section>
          <EvidenceSlot
            slot={slot('decisions')} section={decisions} loading={snap.loading} now={now} styles={styles} colors={colors} moreWhere="the Decision Inbox" onRetry={snap.retry}
            renderItems={(items) => (
              <ul style={styles.list}>
                {items.map((d) => (
                  <li key={d.id} style={styles.row} data-record={d.id}>
                    <span style={styles.para}>{d.headline}</span>
                    <span style={styles.optionMeta}>{d.goalTitle ?? d.decisionKind.replace(/_/g, ' ')} · {ageLabel(d.createdAt, now)}</span>
                    <span><Button colors={colors} variant="bare" flashSuccess={false} onClick={() => { openLink(d.link); }}>Review in Decision Inbox</Button></span>
                  </li>
                ))}
              </ul>
            )}
          />
          <EvidenceSlot
            slot={slot('steward_spend')} section={spend} loading={snap.loading} now={now} styles={styles} colors={colors} collapsible={compact} onRetry={snap.retry}
            renderItems={(items) => items.map((sp, i) => (
              <div key={i} style={styles.column} data-lower-bound={sp.lowerBound ? 'true' : 'false'}>
                <p style={styles.strong}>{sp.lowerBound ? 'At least ' : ''}{usd(sp.costUsd)} · {sp.callCount} call(s){sp.estimatedCallCount ? `, ${sp.estimatedCallCount} estimated` : ''}</p>
                <p style={styles.note}>{sp.scope}</p>
                {sp.note && <p style={styles.note}>{sp.note}</p>}
              </div>
            ))}
          />
          <EvidenceSlot
            slot={slot('skills')} section={skills} loading={snap.loading} now={now} styles={styles} colors={colors} collapsible={compact} moreWhere="Skills" onRetry={snap.retry}
            renderItems={(items) => (
              <>
                <p style={styles.note}>Saved, proposed and successfully exercised are different claims.</p>
                <ul style={styles.list}>
                  {items.map((k) => (
                    <li key={k.id} style={styles.row} data-record={k.id}>
                      <span style={styles.para}>{k.name}</span>
                      <span style={styles.optionMeta}>{k.status ?? 'status not reported'}{k.usageCount !== null ? ` · used ${k.usageCount}×` : ''}{k.lastRun ? ` · last run ${ageLabel(k.lastRun, now)}` : ' · no recorded run'}</span>
                    </li>
                  ))}
                </ul>
              </>
            )}
          />
        </div>
      </div>
    </PlacePanel>
  );
}
