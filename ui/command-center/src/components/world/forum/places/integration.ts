/**
 * World W2 — the integration contract for the place panels (consumed by W6).
 *
 * The places are isolated components. They never import the store, the API
 * client or `askFromForum`; everything that reaches outside the panel comes in
 * through these props, so the integrator decides how each seam is wired and a
 * test can prove that nothing sends on its own (places.contract.test.ts).
 *
 * What the HOST supplies (W6):
 * - `loader`      — ONE `createWorldSnapshotLoader(createApiEvidenceClient(apiFetch))`
 *                   for the whole World (W1). Reads only.
 * - `navigation`  — `createStoreNavigation(useCommandCenter.getState, navigateToTool)` (W1).
 *                   Opening a record only navigates; it never approves or answers.
 * - `drafts`      — `createDraftStore(worldStorage())` (W1). Local only.
 * - `noteWriter`  — `createApiNoteWriter(apiFetch)` (W1). Called ONLY from an
 *                   explicit "Save to project" button, through
 *                   `saveFieldNoteToProject`.
 * - `onRequestSend` — implemented BY THE HOST, normally with
 *                   `askFromForum(req.question, req.agentName, req.criteria, undefined, req.intent)`.
 *                   It is the only path by which a place can send a message,
 *                   and it is called only from the confirmation step's own
 *                   "Send" button — never by selection, typing, restore or mount.
 * - `projectId`, `projects`, `selectedAgentId` — current context. Choosing a
 *                   project inside a place is local and emits `project_changed`.
 * - `onClose`     — the host unmounts the panel. Focus returns to
 *                   `returnFocusTo` (or the element focused when the panel opened).
 *
 * Escape: the panel root is a non-modal `role="dialog"`. ForumView's window
 * Escape handler already ignores keys whose target is inside a dialog, so the
 * panel's own React handler peels ITS layers (confirm → composer → panel) and
 * stops propagation. The places install no global listeners.
 */
import type { WorldNavigation, OpenResult } from '../data/recordLinks';
import type { WorldSnapshotLoader } from '../data/snapshotLoader';
import type { WorldDraftStore } from '../data/drafts';
import type { WorldNoteWriter } from '../data/evidenceClient';
import type { WorldPlaceId } from '../data/identity';

/** The places this lane builds, in the order of the five-minute loop. */
export const W2_PLACE_IDS = ['commons', 'gallery', 'maker'] as const satisfies readonly WorldPlaceId[];
export type W2PlaceId = (typeof W2_PLACE_IDS)[number];

export type SendIntent = 'query' | 'job' | 'capability';

/** What a place hands the host when the user confirms a send. */
export interface PlaceSendRequest {
  placeId: WorldPlaceId;
  projectId: string | null;
  /** World roster id selected for context. Context only — never an assignment. */
  agentWorldId: string;
  /** Display name to pass as `askFromForum`'s agentName. */
  agentName: string;
  intent: SendIntent;
  /** composeBriefRequest(draft).question */
  question: string;
  /** composeBriefRequest(draft).criteria */
  criteria: string;
  /** drafts.ts draftKey(place, project, agent) of the draft being sent. */
  draftKey: string;
}

/**
 * What the host reports back after the send resolved. Report only what is
 * known: a goal is linked only once its id is known; a reply without a
 * tracked job leaves `goal` null and the place says so.
 */
export interface PlaceSendAck {
  /** Plain sentence shown to the user, e.g. "Sent to the orchestrator in the shared conversation." */
  acknowledgement: string;
  conversation?: { sessionId: string } | null;
  goal?: { projectId: string; goalId: string } | null;
}

export interface PlaceProject {
  id: string;
  name: string;
}

/** Local actions the host performs (camera, seating, other panels). */
export type PlaceLocalAction = 'sit_together' | 'open_profile' | 'explore_places' | 'try_example';

/** Everything a place tells the host. None of these is a send. */
export type PlaceEvent =
  | { type: 'place_opened'; placeId: WorldPlaceId }
  | { type: 'place_closed'; placeId: WorldPlaceId }
  | { type: 'local_action'; placeId: WorldPlaceId; action: PlaceLocalAction; agentWorldId?: string }
  | { type: 'project_changed'; placeId: WorldPlaceId; projectId: string | null }
  | { type: 'record_selected'; placeId: WorldPlaceId; recordKey: string }
  | { type: 'record_opened'; placeId: WorldPlaceId; recordKey: string; result: OpenResult }
  | { type: 'draft_changed'; placeId: WorldPlaceId; draftKey: string }
  | { type: 'send_requested'; placeId: WorldPlaceId; draftKey: string }
  | { type: 'send_acknowledged'; placeId: WorldPlaceId; draftKey: string; ack: PlaceSendAck }
  | { type: 'send_failed'; placeId: WorldPlaceId; draftKey: string; reason: string }
  | { type: 'note_saved'; placeId: WorldPlaceId; projectId: string; noteId: string }
  | { type: 'note_save_failed'; placeId: WorldPlaceId; reason: string };

export interface PlaceHostProps {
  loader: WorldSnapshotLoader;
  navigation: WorldNavigation;
  drafts: WorldDraftStore;
  noteWriter: WorldNoteWriter;
  /** Project currently in context, or null (never guessed). */
  projectId: string | null;
  /** Projects the user may choose for a place or a field note. */
  projects: readonly PlaceProject[];
  /** World roster id of the agent currently selected (context for briefs). */
  selectedAgentId: string;
  /** The ONLY way a place sends a message. Host-implemented. */
  onRequestSend(request: PlaceSendRequest): Promise<PlaceSendAck>;
  onClose(): void;
  onEvent?(event: PlaceEvent): void;
  /** Where focus goes when the panel closes. Defaults to the element focused at open. */
  returnFocusTo?: HTMLElement | null;
  /** Display names by World roster id (the orchestrator's configured name, etc.). */
  agentNames?: Readonly<Record<string, string>>;
  /** Logical size the panel is laid out in; defaults to the window. ≤ 720 tall or < 1100 wide is compact. */
  viewport?: { width: number; height: number };
  /** Clock for freshness labels (tests and fixtures pin it). */
  now?: () => number;
  /** Id factory for local field notes. */
  newId?: () => string;
}
