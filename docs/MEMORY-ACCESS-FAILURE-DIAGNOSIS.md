# Henry Story-Memory Failure: Evidence and Repair Contract

**Incident:** session `20260904_2`, 2026-09-04 00:40–01:31 ADT  
**Authoritative source:** session `20260901_18`, message `20965`  
**Status:** the source was preserved; Henry's claims that it was absent or lost were false.

## Proven facts

The detailed story turn exists verbatim in three durable forms:

1. The original session transcript (`20260901_18`, message `20965`).
2. Approved Personal project note
   `decision-note:01a062f4-1636-77f0-abea-715cffdf85b9`.
3. Brain memories:
   - approved note: `0972162ee274e0ec`
   - exact chat turn: `acae4c336a100b61`

The causal order that a replay must preserve is:

`flaming arrow → tree beacon → apples → camp → music clears fog → pie → canyon exit`

During the incident, Brain recorded 21 cascade searches for session `20260904_2`.
The approved note appeared in 11 result sets and the exact chat memory in 12.
They reached ranks 3 and 2 respectively. They were therefore retrieved; the
agent-facing search contract failed to expose their source content reliably.

## Root causes

### 1. Breadth-first abstraction consumed the search budget

`search_memory` takes up to eight hits and passes them to a 480-token context
budget. The assembler first writes an abstract for every hit, then attempts to
deepen results. Long Librarian descriptions can consume the entire budget before
any exact source text is returned.

The relevant descriptions mentioned the broad battle and character encounter
but omitted the arrow, beacon, apples, camp, music, pie, and their order.

### 2. Search hid the stable identity needed for exact retrieval

The internal hit retains a memory ID/key, but the rendered tool output prints
only layer, score, and text. Permagent's Brain wrapper already supports exact
`get_memory`/`get_memory_by_key`; Henry had no tool that exposed that operation.

### 3. Session navigation was lossy

The Orchestrator's `view_session` supported only first/last messages or a brief
LLM summary. It offered no query-within-session, exact message lookup, or
pagination. Henry consequently searched rotated request logs and guessed session
IDs instead of querying the authoritative session database.

### 4. Project and story scope were not available to recall

The source and incident sessions have no project hint. Their chat memories were
honestly marked `unverifiable` and stored in the general wing. The approved note
is linked to Personal in Permagent, but ordinary Brain recall does not consume
that separate project association.

### 5. The agent made epistemically invalid absence claims

Failed queries, an incorrect database path, and empty command output were treated
as proof of nonexistence. A failed or incomplete lookup proves only that the
source was not retrieved by that attempt. The agent then verified a document it
had just reconstructed rather than checking each claim against a source ID.

### 6. Tool loops inflated the transcript and cost

The incident session contains 394 persisted message rows and about 2.62 million
serialized characters. Oversized shell results were correctly reduced to a
bounded model-visible preview, but their full duplicate `structuredContent`
remained embedded in stored messages. This is a persistence/rehydration and
latency hazard. The repeated search/guess/rewrite cycle also fragmented the
active conversation and repeatedly crossed the review budget.

### 7. Screenshot ingestion was not durable

The screenshot message (`21613`) contains only a generated OCR sentence. It has
no image content block, and no attachment row was created. Sparse OCR therefore
became an irreversible dead end: the original pixels could not be reopened or
reprocessed.

### 8. Chat memory capture has a separate durability risk

Chat-to-Brain persistence runs in a detached task. That did not cause this
incident, because the relevant memories exist, but a crash can leave a session
turn committed without a retryable Brain capture receipt. Detached completion
can also assign storage time rather than original conversational order.

## Repair invariants

1. Search results expose stable memory/source identity and a matched source
   excerpt.
2. A trusted top hit receives exact-source budget before lower-ranked abstracts.
3. Every returned memory can be opened exactly by ID/key.
4. Sessions can be searched and paged without an LLM summary.
5. Long-form narrative recall expands a trusted source into bounded, ordered
   neighboring evidence with provenance.
6. Session persistence durably enqueues idempotent Brain capture using original
   time and ordinal.
7. “Not stored” is permitted only after a successful authoritative lookup;
   otherwise the agent says “not retrieved from the available record.”
8. Verification checks source IDs and chronology, not merely the generated file.
9. Dropped images receive a durable attachment ID; partial OCR and original
   pixels travel together and OCR can be retried.
10. Verification retries are bounded: a second unchanged failure escalates with
    evidence instead of repeating the same operation.

## Spectral boundary

Spectral remains the only semantic memory library, index, and recall engine.
The original session transcript remains authoritative evidence. A Permagent
outbox may hold an unsearchable turn only until Spectral confirms ingestion; it
is transport state, not a second memory corpus. Successful delivery removes the
payload (or retains only a compact receipt), and every recall path continues to
query Spectral.

Spectral already provides exact memory lookup, recognition context, episode
membership, and source content. The observed incident is repaired through thin
Permagent adapters over those APIs plus exact access to the original
session/attachment evidence. No competing embeddings, semantic tables, or
ranking logic may be added. Spectral changes are justified only if the
deterministic replay still fails after these adapter, session-navigation,
capture, and story-expansion contracts are corrected.

## Golden replay acceptance

Use a synthetic long-form story fixture with the same failure shape. The replay
must prove:

- a paraphrased character-origin question returns the transformation and rescue
  facts without contradiction;
- a meeting question returns every ordered causal beat, not only an abstract;
- an uncertain backstory remains explicitly uncertain;
- a later chapter remains distinct from the earlier encounter;
- a weak-OCR screenshot stays attached and can be reprocessed;
- every synthesized claim cites a memory, session/message, or attachment ID;
- no failed lookup is converted into a claim that data was never stored;
- the run remains within fixed tool-call, transcript-size, and retry bounds.
