/**
 * World W1 — the six-place interaction schema, as data for W2 to render.
 *
 * Each place has ONE recognizable interaction object and ONE primary useful
 * action (Astra ruling §1). Selecting the object opens an accessible DOM
 * panel; the Places list exposes the same actions without walking, pointer
 * lock, precise aiming or camera travel (`accessible`).
 *
 * Action kinds and the send rule:
 * - `local`: seating, camera, opening a panel, choosing a project. No network write.
 * - `navigate`: opens a record's home view (recordLinks.ts). Never approves.
 * - `prepare`: edits a local draft or note (drafts.ts). Never sends.
 * - `explicit_send`: sends through askFromForum, ONLY from its own button.
 * - `explicit_save`: saves a field note to a chosen project, ONLY from its own button.
 * `sends` is true exactly for the explicit kinds; no primary action sends.
 */
import type { WorldPlaceId } from './identity';
import type { WorldRecordTarget } from './recordLinks';
import type { ContractId, EvidenceSlotId } from './types';

export type PlaceActionKind = 'local' | 'navigate' | 'prepare' | 'explicit_send' | 'explicit_save';

export interface PlaceAction {
  id: string;
  label: string;
  kind: PlaceActionKind;
  /** One sentence for the accessible description and tooltip. */
  description: string;
  /** True only for explicit_send / explicit_save. */
  sends: boolean;
  /** Home view a navigate action opens. */
  target?: WorldRecordTarget;
  /** Precondition the UI must satisfy before enabling the action. */
  requires?: 'project' | 'selected_record' | 'draft_text' | 'note_text';
  /** A clearly labelled demonstration, separate from real evidence. */
  demonstration?: boolean;
}

export interface EvidenceSlot {
  id: EvidenceSlotId;
  label: string;
  contract: ContractId;
  /** Honest copy for an empty source (what it does NOT mean, too). */
  emptyCopy: string;
}

export interface PlaceSchema {
  id: WorldPlaceId;
  name: string;
  /** World roster id of the place's host. */
  host: string;
  interactionObject: string;
  primaryAction: PlaceAction;
  secondaryActions: PlaceAction[];
  evidence: EvidenceSlot[];
  /** Shown when every evidence slot is empty or setup-required. */
  emptyState: { title: string; body: string };
  /** The non-spatial equivalent in the Places list. */
  accessible: { listLabel: string; description: string; shortcutHint: string };
  learningOutcome: string;
}

const local = (id: string, label: string, description: string, extra: Partial<PlaceAction> = {}): PlaceAction => ({ id, label, kind: 'local', description, sends: false, ...extra });
const nav = (id: string, label: string, target: WorldRecordTarget, description: string, extra: Partial<PlaceAction> = {}): PlaceAction => ({ id, label, kind: 'navigate', target, description, sends: false, ...extra });
const prep = (id: string, label: string, description: string, extra: Partial<PlaceAction> = {}): PlaceAction => ({ id, label, kind: 'prepare', description, sends: false, ...extra });
const send = (id: string, label: string, description: string): PlaceAction => ({ id, label, kind: 'explicit_send', description, sends: true, requires: 'draft_text' });
const save = (id: string, label: string, description: string): PlaceAction => ({ id, label, kind: 'explicit_save', description, sends: true, requires: 'note_text' });

export const PLACE_SCHEMAS: readonly PlaceSchema[] = [
  {
    id: 'commons',
    name: 'Arrival table',
    host: 'henry',
    interactionObject: 'The work folio on the arrival table',
    primaryAction: prep('prepare_brief', 'Prepare a brief', 'Write the outcome, context, constraints and what would count as done. Nothing is sent until you choose Send.'),
    secondaryActions: [
      local('sit_together', 'Sit together', 'Changes seating and camera only. Voice is a separate control; text stays available.'),
      send('send_request', 'Send to orchestrator', 'Sends the prepared brief to the orchestrator in the shared conversation. Selecting an agent adds context; it does not assign them.'),
      nav('review_decision', 'Review a decision', 'decision_inbox', 'Opens the Decision Inbox. Nothing is approved from the World.'),
      nav('resume_conversation', 'Resume conversation', 'conversation', 'Opens the canonical shared chat. Offered only when a real session exists.', { requires: 'selected_record' }),
    ],
    evidence: [
      { id: 'orchestrator', label: 'Orchestrator status', contract: 'C3', emptyCopy: 'Orchestrator status could not be read.' },
      { id: 'active_goals', label: 'Goals in flight', contract: 'C5', emptyCopy: 'No goal is ready or executing.' },
      { id: 'decisions', label: 'Waiting for your review', contract: 'C5', emptyCopy: 'Nothing is waiting for your review.' },
      { id: 'requests', label: 'Recent requests', contract: 'C3', emptyCopy: 'No recent requests. A failed or cancelled request would be listed here.' },
    ],
    emptyState: { title: 'Nothing is in progress yet', body: 'Prepare a brief to start. It stays a draft until you send it.' },
    accessible: { listLabel: 'Arrival table — prepare a brief', description: 'Meet the orchestrator, see what is in flight, and prepare a request with a definition of done.', shortcutHint: 'Places list → Arrival table → Prepare a brief' },
    learningOutcome: 'Explain which constraint changes the plan, and tell a prepared brief from a submitted request.',
  },
  {
    id: 'gallery',
    name: 'Garden archive',
    host: 'librarian',
    interactionObject: 'The open source folio beside the Librarian',
    primaryAction: local('browse_memories', 'Browse memories', 'Opens the real memory folio with project and search controls. Demonstrations are a separate entry.'),
    secondaryActions: [
      nav('open_in_brain', 'Open in Brain', 'brain', 'Opens the selected memory in Brain, where correction and forgetting live.', { requires: 'selected_record' }),
      prep('ask_about_evidence', 'Ask about this evidence', 'Prepares a question that references the selected records. Not sent until you choose Send.', { requires: 'selected_record' }),
      save('save_field_note', 'Save field note', 'Saves your note to the project you choose. Nothing is saved automatically.'),
      local('try_example', 'Try an example', 'Opens the fictional evidence exercise. It is a demonstration, not your memories.', { demonstration: true }),
    ],
    evidence: [
      { id: 'memories', label: 'Remembered records', contract: 'C4', emptyCopy: 'Nothing has been remembered yet.' },
      { id: 'librarian_activity', label: 'Librarian activity', contract: 'C2', emptyCopy: 'Nothing attributed to the Librarian in this window. That does not prove it was inactive.' },
      { id: 'notes', label: 'Project notes', contract: 'C4', emptyCopy: 'No notes on this project yet.' },
    ],
    emptyState: { title: 'The archive is empty', body: 'Memories appear after conversations, notes or documents are saved. Try the example to see how evidence is inspected.' },
    accessible: { listLabel: 'Garden archive — browse memories', description: 'Inspect what is remembered, where it came from, and reopen it in Brain.', shortcutHint: 'Places list → Garden archive → Browse memories' },
    learningOutcome: 'Distinguish stored context, source evidence and your own inference, then reopen the same record.',
  },
  {
    id: 'maker',
    name: 'Maker workshop',
    host: 'steward',
    interactionObject: 'The task folio on the workbench',
    primaryAction: nav('inspect_work', 'Inspect current work', 'goal', 'Opens the real goal behind the selected work item.', { requires: 'selected_record' }),
    secondaryActions: [
      nav('compare_result', 'Compare request and result', 'goal', 'Opens the goal detail, where acceptance criteria, result and evidence sit together.', { requires: 'selected_record' }),
      nav('review_required', 'Review required decision', 'decision_inbox', 'Opens the Decision Inbox. Nothing is approved from the World.'),
      prep('prepare_repair', 'Prepare a repair brief', 'Drafts a repair or capability request. Not sent until you choose Send.'),
      nav('inspect_skill', 'Inspect a skill', 'skills', 'Opens Skills: saved, proposed and exercised are different claims.'),
      nav('open_build', 'Open Build', 'build', 'Opens Build through the app navigation.'),
    ],
    evidence: [
      { id: 'steward_goals', label: 'Steward goals', contract: 'C2', emptyCopy: 'No goals are owned by the Steward.' },
      { id: 'steward_activity', label: 'Steward activity', contract: 'C2', emptyCopy: 'Nothing attributed to the Steward in this window.' },
      { id: 'steward_spend', label: 'Attributed spend', contract: 'C2', emptyCopy: 'No spend attributed to the Steward.' },
      { id: 'decisions', label: 'Needs review', contract: 'C5', emptyCopy: 'Nothing is waiting for your review.' },
      { id: 'skills', label: 'Saved skills', contract: 'C6', emptyCopy: 'No skills are saved yet.' },
    ],
    emptyState: { title: 'No work has been attributed here yet', body: 'A completed result appears on the shelf only when a real result record exists.' },
    accessible: { listLabel: 'Maker workshop — inspect current work', description: 'Follow work to its result, see what still needs review, and inspect saved skills.', shortcutHint: 'Places list → Maker workshop → Inspect current work' },
    learningOutcome: 'Identify what would count as done, inspect the returned evidence, and recognize when review is still required.',
  },
  {
    id: 'council_garden',
    name: 'Observatory',
    host: 'forecaster',
    interactionObject: 'The instrument desk beneath the dome',
    primaryAction: local('inspect_forecast', 'Inspect observation versus forecast', 'Shows observed history beside the forecast or refusal, with method, coverage and freshness.', { requires: 'project' }),
    secondaryActions: [
      local('choose_signal', 'Choose a project and signal', 'Selects which project series the instrument shows.'),
      local('compare_council', 'Compare a Council report', 'Shows an existing Council report with its disagreements visible.'),
      prep('what_would_change', 'Prepare “What would change this forecast?”', 'Drafts the question. Not sent until you choose Send.'),
      save('save_hypothesis', 'Save a hypothesis', 'Saves your hypothesis as a note on the project you choose.'),
    ],
    evidence: [
      { id: 'forecasts', label: 'Observations and forecasts', contract: 'C7', emptyCopy: 'No series is bound to this project. That is not a flat trend.' },
      { id: 'council', label: 'Council reports', contract: 'C7', emptyCopy: 'No Council session has finished yet.' },
      { id: 'forecaster_activity', label: 'Forecaster activity', contract: 'C2', emptyCopy: 'Nothing attributed to the Forecaster in this window.' },
    ],
    emptyState: { title: 'No forecast to examine yet', body: 'Choose a project with a bound series. A refusal to forecast is a real answer and is shown with its reason.' },
    accessible: { listLabel: 'Observatory — inspect a forecast', description: 'Separate observations from projections and see why a forecast was refused.', shortcutHint: 'Places list → Observatory → Inspect observation versus forecast' },
    learningOutcome: 'Separate observations from projections and name an uncertainty or reason for refusal.',
  },
  {
    id: 'observatory_garden',
    name: 'Observatory garden',
    host: 'forecaster',
    interactionObject: 'The telescope and the field notebook on its plinth',
    primaryAction: prep('review_hypothesis', 'Review my hypothesis', 'Opens your saved hypothesis and adds “What would change my mind?”. Not sent or saved until you choose.'),
    secondaryActions: [
      local('look_through_telescope', 'Look through the telescope', 'A bounded, interruptible local camera view. Not live astronomy.'),
      local('inspect_sky', 'Inspect this sky', 'Explains the survey backdrop, artistic orientation and illustrative planets.'),
      nav('sit_and_ask', 'Sit and ask', 'conversation', 'Continues the existing conversation.', { requires: 'selected_record' }),
      local('return_observatory', 'Return to Observatory', 'Moves back to the Observatory desk.'),
      local('return_arrival', 'Return to Arrival', 'Moves back to the arrival table.'),
      save('save_hypothesis_note', 'Save to project', 'Saves the revised hypothesis as a note on the project you choose.'),
    ],
    evidence: [
      { id: 'hypotheses', label: 'Saved hypotheses', contract: 'C4', emptyCopy: 'No hypothesis saved yet. Save one from the Observatory.' },
      { id: 'sky', label: 'About this sky', contract: 'C7', emptyCopy: 'Sky provenance is unavailable.' },
    ],
    emptyState: { title: 'A quiet place to reflect', body: 'Bring a hypothesis from the Observatory, or look through the telescope.' },
    accessible: { listLabel: 'Observatory garden — review a hypothesis', description: 'Reflect on a forecast hypothesis and learn what the sky view does and does not show.', shortcutHint: 'Places list → Observatory garden → Review my hypothesis' },
    learningOutcome: 'Distinguish visual representation from measured data and formulate a useful follow-up question.',
  },
  {
    id: 'conservatory',
    name: 'Conservatory',
    host: 'growth_measurement',
    interactionObject: 'The measurement folio on the potting bench',
    primaryAction: local('inspect_intervention', 'Inspect an intervention', 'Shows a measured change with its baseline, window, verdict and confounders.'),
    secondaryActions: [
      local('compare_before_after', 'Compare before and after', 'Shows denominators and dates for the selected intervention.', { requires: 'selected_record' }),
      prep('explain_verdict', 'Explain this verdict', 'Prepares a question with the selected evidence. Not sent until you choose Send.', { requires: 'selected_record' }),
      nav('open_grow', 'Open Grow', 'grow', 'Opens Grow to verify or manage an action through existing controls.'),
      local('water_demo', 'Try the water demonstration', 'A demonstration, clearly separate from real measurements.', { demonstration: true }),
      save('save_observation', 'Save an observation', 'Saves your observation as a note on the project you choose.'),
    ],
    evidence: [
      { id: 'growth_results', label: 'Measured interventions', contract: 'C8', emptyCopy: 'No intervention has been measured yet. Implemented is not the same as helped.' },
      { id: 'grower_state', label: 'Grower availability', contract: 'C1', emptyCopy: 'Grower configuration could not be read.' },
      { id: 'grower_activity', label: 'Grower activity', contract: 'C2', emptyCopy: 'Nothing attributed to the Grower in this window.' },
    ],
    emptyState: { title: 'Nothing has been measured yet', body: 'A lack of analytics is a setup state, not zero growth. Plant health here is scenery, not project performance.' },
    accessible: { listLabel: 'Conservatory — inspect an intervention', description: 'Learn whether a change helped, hindered, had no effect, or was inconclusive.', shortcutHint: 'Places list → Conservatory → Inspect an intervention' },
    learningOutcome: 'Explain why implementation alone does not establish benefit.',
  },
];

export function placeSchema(id: WorldPlaceId): PlaceSchema {
  const hit = PLACE_SCHEMAS.find((p) => p.id === id);
  if (!hit) throw new Error(`No World place schema for ${id}`);
  return hit;
}
