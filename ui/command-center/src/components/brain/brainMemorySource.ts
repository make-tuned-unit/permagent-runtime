/**
 * Brain memory source & forget — pure helpers shared by BrainList and
 * BrainView's memory side panel (R5 "a memory shows its source"; "explicit
 * forget with undo").
 *
 * The vocabulary here mirrors `permagent::brain_feed::FeedSource` (Rust,
 * `crates/goose/src/brain_feed.rs`) — the closed set of `RememberOpts.source`
 * tags a memory can carry. It is intentionally NOT fetched from
 * `GET /api/brain/feed-status` (which already exposes a label per source for
 * Settings → Memory): that panel's labels are plural category names
 * ("Conversations", "Cards and tasks") meant for a settings row, while a
 * per-memory chip reads better as a short "from a …" phrase. Duplicating a
 * small, static string table here avoids a second network dependency for a
 * few-dozen-byte lookup, and both sides are typed against the same Rust enum
 * so a source added later shows as "from <humanized tag>" (never a raw
 * identifier, per R3) until this map is updated.
 */

/** `RememberOpts.source` tag → the phrase a source Chip reads. */
const SOURCE_PHRASES: Record<string, string> = {
  chat: 'from a conversation',
  'permagent.owner': 'from your profile',
  'permagent.person': 'from a relationship update',
  'user.answer': 'from your answer',
  'permagent.goal': 'from a goal',
  'permagent.decision.asked': 'from a decision',
  'permagent.decision.auto': 'from a decision',
  'permagent.card': 'from a task',
  'permagent.project': 'from a project',
  'permagent.meeting': 'from a meeting',
  'permagent.growth': 'from a growth action',
  'permagent.analytics': 'from analytics',
  'permagent.finance': 'from a finance fact',
  'permagent.watcher_insight': 'from the Watcher',
  'permagent.council': 'from the Council',
  'permagent.briefing': 'from a briefing',
  'permagent.skill': 'from a skill',
  'permagent.lesson': 'from a lesson',
  'permagent.incident': 'from an incident',
  'permagent.model_review': 'from a model review',
};

/** Short plural label for the List's "sources" filter chip row — a category
 *  name, not the per-row phrase (a filter chip reads as a noun, not a
 *  sentence fragment). Order matches `FeedSource::ALL` (Settings → Memory). */
export const SOURCE_FILTERS: { tag: string; label: string }[] = [
  { tag: 'chat', label: 'conversations' },
  { tag: 'permagent.owner', label: 'your profile' },
  { tag: 'permagent.person', label: 'relationships' },
  { tag: 'user.answer', label: 'your answers' },
  { tag: 'permagent.goal', label: 'goals' },
  { tag: 'permagent.decision.asked', label: 'decisions asked' },
  { tag: 'permagent.decision.auto', label: 'decisions (auto)' },
  { tag: 'permagent.card', label: 'tasks' },
  { tag: 'permagent.project', label: 'projects' },
  { tag: 'permagent.meeting', label: 'meetings' },
  { tag: 'permagent.growth', label: 'growth' },
  { tag: 'permagent.analytics', label: 'analytics' },
  { tag: 'permagent.finance', label: 'finance' },
  { tag: 'permagent.watcher_insight', label: 'watcher insights' },
  { tag: 'permagent.council', label: 'council' },
  { tag: 'permagent.briefing', label: 'briefings' },
  { tag: 'permagent.skill', label: 'skills' },
  { tag: 'permagent.lesson', label: 'lessons' },
  { tag: 'permagent.incident', label: 'incidents' },
  { tag: 'permagent.model_review', label: 'model reviews' },
];

/** The sentinel the backend's `?source=` filter treats as "no source
 *  recorded" (an `IS NULL` check — never a real `FeedSource` tag). */
export const UNKNOWN_SOURCE = 'unknown';

/** Humanize a raw, dotted/underscored tag the map doesn't know about yet,
 *  e.g. `permagent.foo_bar` → `foo bar` — never render the identifier itself
 *  (R3: no snake_case/dotted internals in user copy). */
function fallbackPhrase(tag: string): string {
  const cleaned = tag.replace(/^permagent\./, '').replace(/[._-]+/g, ' ').trim();
  return cleaned ? `from ${cleaned}` : 'from an unlabeled source';
}

/** A memory's source, humanized for its Chip. `null`/`undefined`/empty is
 *  honest "source unknown" — never guessed (R5). */
export function humanizeMemorySource(source: string | null | undefined): { phrase: string; known: boolean } {
  if (!source) return { phrase: 'source unknown', known: false };
  const phrase = SOURCE_PHRASES[source];
  return phrase ? { phrase, known: true } : { phrase: fallbackPhrase(source), known: true };
}

/**
 * Whether a memory's `key` can be forgotten via `DELETE /api/brain/memory`.
 *
 * `null`/empty means no key at all (a raw `/api/brain/search` "memory"-bucket
 * hit never carries one — see `searchResultToGraphMemory`). A key starting
 * `chat:` is that same search path's SYNTHETIC placeholder for a chat hit
 * (`chat:${session_id}`) — not a real Spectral key, which chat persistence
 * writes as `chat-{session_id}-{turn_ordinal}` (hyphen, not colon; see
 * `crates/goose-server/src/brain_ops.rs`). Forgetting the placeholder would
 * either no-op (existed: false) or, worse, never resolve — so the action is
 * withheld rather than offered and silently failing.
 */
export function isForgettableMemoryKey(key: string | null | undefined): key is string {
  return !!key && !key.startsWith('chat:');
}
