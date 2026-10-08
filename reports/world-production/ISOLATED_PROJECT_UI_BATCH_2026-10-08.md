# Next isolated project/Brain acceptance batch — October 8

Prepared from source; no UI, runtime/API writes or builds were performed by this worker. The coordinator is the sole CUA operator. Run against the paired isolated browser **5302 → daemon 3002** after confirming its bounded runtime is ready. Keep original meeting evidence notes/cards intact. Use a uniquely named disposable plain card and plain note within an existing synthetic QA project.

Instruction audit: no `AGENTS.md` exists in this checkout's ancestor chain or command-center subtree. The `ui/goose2` and `ui/text` instruction files do not govern these components.

## Ordered CUA steps

| Step | Action and evidence to retain | Source/constraint |
| --- | --- | --- |
| 1 | Open a known isolated QA project. Record project name, original summary and board counts; visit Overview, Details, Kanban and back. | `ProjectWorkspace.tsx`, `ProjectOverview.tsx`; covers parts of P1–P4. |
| 2 | Overview Summary → Edit. Add a draft marker, Discard, reopen and confirm original text. Then save a reversible QA description/brief, revisit to prove persistence, and restore the original. | `ProjectOverview.tsx:155` initializes draft on Edit and calls existing project PATCH. This is project update coverage, not project creation/deletion. |
| 3 | Kanban Backlog → + Add card. Enter a draft, Cancel or Escape; count stays unchanged. Reopen and add `UI QA disposable 2026-10-08`. | `ProjectsView.tsx:756` creates a plain card in ordinary columns. **Do not add into a Triage state-bound column:** that path creates a roadmap goal. |
| 4 | Open only the new card → Edit title/description/date → Save. Reopen and compare persisted values. Try blank title and expect inline “A card needs a title.”; Discard restores saved values. Clear the date and verify removal. | `CardDetailModal.tsx:103`: separate PATCH title/description and PUT due-date, then authoritative GET. No worker launch is involved for a plain card. |
| 5 | Drag only that plain card through ordinary Backlog/Doing/Done and back; verify one card, correct column counts, no generated worker/goal state. Delete it via its menu and verify original board counts. | `ProjectsView.tsx:698`, `:784`; plain card update differs from guarded goal transitions in `crates/goose/src/cards.rs:1766`. Skip if its type or board binding is unclear; do not manipulate meeting evidence or goals. |
| 6 | Details → Notes. Empty/whitespace body keeps Save disabled. Add a plain note titled `UI QA note 2026-10-08`, body `Synthetic UI check. No purchase or email is authorized. Final tail: the review stays pending.` Save, revisit, expand and compare complete text. | `NotesPanel.tsx:133`; ordinary note has no meeting kind/recording ID and does not request meeting triage. It does feed the isolated Brain. |
| 7 | View in Brain. Verify exact body/tail, source disclosure and honest graph/preview state. Return to project Memories, refresh and inspect linked row; do not assume description-less memories appear in default graph. | Existing memory-key and project-memory ID contracts are below. This is BR1/BR2 plus project note/Brain-loop coverage. |
| 8 | Return to Notes, delete only the disposable note, revisit and verify it disappeared. Inspect Memories refresh separately; do not equate note deletion with complete Brain forgetting. | `NotesPanel.tsx:159` directly deletes; no confirmation is implied. Brain deletion/undo requires its own explicitly scoped fixture test (BR3). |

No project create/delete affordance was found in the inspected Projects UI. Therefore this batch covers project read/update plus card and note lifecycle, **not complete project CRUD**. It does not start goals, publish content, connect services or alter external accounts. Record before/after screenshots and actual UI outcomes; checklist steps alone are not passes.

## Brain source diagnosis and proposed bounded fix

`NotesPanel.tsx:175` passes the saved `memory_key` plus note text to `focusBrainMemory`. `MemoriesPanel.tsx:73` passes the actual Spectral ID and stable key. `resolveFocusedMemory` matches by ID, then key. No key/ID mismatch is established.

The default graph deliberately selects only memories with non-null descriptions, capped at 100 (`crates/goose-server/src/routes/brain.rs:636`). Fresh notes can therefore exist durably outside that graph. Preview synthesis omits source because the note/project-memory wire shapes do not include it. BrainView also consumes the navigation request immediately from a preview while the graph is still loading; it does not later resolve that preview from the authoritative memory endpoint.

The already implemented **GET `/api/brain/memory?key=`** (`brain.rs:1325`) returns full `GraphMemory`, including actual ID, key, source, timestamps and reinforcement metadata without requiring graph enrichment. Proposed frontend-only repair: read that endpoint for the selected keyed preview, validate its identity, ignore stale replies after selection changes/unmount, and replace the preview with authoritative saved data. Label a resolved memory outside the graph honestly; do not invent graph placement or a source value. Preserve a truthful preview fallback when missing/unavailable. Test success, failure/mismatch and late-response selection safety. Coordinate file ownership with the parent before edits.

## Implemented Brain resolution — source ready for CUA

The coordinator approved the bounded frontend change. `BrainView.tsx` now resolves the selected keyed preview with the existing GET memory endpoint, validates the returned key and any known ID, and carries the actual source/full text into the side panel. A resolved row absent from the current graph says **Saved memory — outside current graph**. Missing, mismatched or unavailable responses retain the original preview and a visible explanation. Selection changes and unmount abort the read and prevent late results from replacing another selection. No graph nodes, backend schema, Record pipeline or write permissions changed.

The focused render also exposed existing unkeyed Tooltip wrappers in memory/entity lists; stable keys now sit on the outer returned wrappers. Final tests passed **42/42** across `BrainView.search`, `BrainView.legend`, `brainMemoryFocus` and `brainFocus.store`; TypeScript and scoped diff whitespace checks passed. Regression cases include exact encoded-key/full-source success, 404, mismatched key, mismatched ID, newer selection and unmount. Source is frozen for coordinator CUA. Browser/native acceptance of this new behavior remains pending; packaged native r3 predates it.
