# Durable meeting-card evidence

Source implementation only. The active `record-r1` native build began before this affordance; a later frontend bundle and native pass are required.

Generated standard cards already persist `source: meeting_triage`, `source_note_id`, `source_recording_id`, `source_triage_id` and `source_item_key` in `crates/goose/src/meeting_triage.rs`. Session IDs are not stored on cards. The existing receipt and session-view routes resolve them; no backend metadata change was needed.

`CardDetailModal.tsx` now shows **Source meeting → View meeting source** for these cards. This survives dismissing the Record receipt and reopening the app because it reads the saved card metadata. Opening the card alone performs no provenance requests. Clicking the affordance performs only:

- `GET /api/projects/{project}/notes`, resolving the stored note ID against the same project's `project_id` (the notes response uses snake_case).
- `GET /api/projects/{project}/recordings/{recording}/triage`, matching receipt ID, project, recording and note (the receipt uses camelCase).

**Open source note** uses the existing `focusProjectNote` path to project Details and its expanded note. **Open triage session** first resolves `GET .../triage/session`, verifies all source identities and the receipt's session ID, then uses the same `switchToSession` seam as Record and opens the chat dock. It sends no chat message and does not execute the generated task.

Missing metadata, unavailable/deleted notes, mismatched receipts, missing sessions and failed reads are explicit states. **Refresh source** repeats reads only. Closing the card while session lookup is pending prevents late navigation. Existing raw metadata remains in Details; no source IDs are invented, and no retry-triage, card-write or execution controls were added.

Validation:

- Initial provenance + existing Kanban-detail run: **13/13 passed**.
- Final provenance run after the dismissal guard: **11/11 passed**; this overlaps the initial run.
- Coverage includes lazy reads, encoded route parameters, project-bound note navigation, actual session resolution, cross-project/source mismatch rejection, incomplete metadata, read retry, absent/deleted/disowned sessions, card-detail integration without Record state, and dismissal during lookup.
- The worker TypeScript run exited 0 before the final asynchronous-dismissal test was incorporated. The coordinator’s later check found TS2322 in that test’s Promise resolver; it is corrected with an explicit `Promise<unknown>`. The subsequent coordinator rerun passed **14/14** provenance/Kanban-detail tests and TypeScript exited 0 (`card-source-tests.log`, `card-source-typecheck.log`). Scoped diff checks were clean. No Cargo, native build, UI control or live requests were performed in this lane.

Native acceptance still needed: reopen a generated card after dismissing Record; inspect source note, return to board, inspect triage session, and exercise an unavailable-source state on a disposable fixture.

The coordinator also added the generated card’s **Source meeting** affordance to the project entry in the agent navigation guide. This documents the source capability; it is not evidence of a native navigation or live-agent pass.
