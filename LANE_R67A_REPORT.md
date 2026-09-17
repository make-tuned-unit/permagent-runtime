# Lane R67a — the E7 call-site completeness gate stops rejecting correct reviews

## The defect

M52 record r67 s1, fixture `harness-refactor-shipping-quote-signature`
(run dir `laneM17/dschat-r67/s1-runs/dschat-harness-refactor-shipping-quote-signature-1789676280712632000-6`).
The harness listed six call sites:

```
src/admin/manualQuote.ts:5, src/admin/manualQuote.ts:10, src/api/quoteHandler.ts:10,
src/cart/cartSummary.ts:7, src/checkout/checkout.ts:12, src/shipping/quote.ts:16
```

Round 1 (minimax) emitted no CALLSITE lines → UNCERTAIN (correct, Lane R66a).
Round 2 escalated to `anthropic/claude-fable-5`, which reviewed the change and
answered four of the six sites — one line per file it considered meaningful.
The other two were `src/admin/manualQuote.ts:5` (where `computeManualQuote` is
DEFINED) and `src/shipping/quote.ts:16` (where `quoteShipping` is DEFINED), so
the round's ONLY findings were the harness's own E7 complaints →
REQUEST_CHANGES → send-back. Round 3 UNCERTAIN again → parked.

Two independent causes:

1. The list included the changed symbol's own definition site. The definitions
   were reflowed (`export function quoteShipping(` with the parameter list on
   the following lines), and `definition_in_line`'s `\(([^)]*)\)` needs the
   closing paren on the same line, so the definition HEAD was not recognized as
   a definition and was listed as a call site of itself. Nothing truthful can
   be said about such a "call site".
2. Accounting required an exact `file:line` match per listed site, while a
   reviewer naturally writes one CALLSITE line per file.

## The rule change (`crates/goose/src/cost_router/call_site_completeness.rs`)

**(a) Definition sites are excluded.** New `pub fn line_defines_symbol(line,
symbol)` is the single recognizer: a complete one-line definition
(`definition_in_line`), a reflowed definition's HEAD line
(`definition_head_symbol`), or a `const`/`let`/`var` binding
(`binding_declares_symbol`, moved out of `orchestrator.rs`).
`enumerate_call_sites` and the live `git grep` seam
(`orchestrator::line_defines_symbol`, used by `call_sites_from_grep` and
`definition_lines_from_grep`) now share it, so both agree on what a definition
is. A file that only defines the symbol therefore has zero call sites.

The head patterns are deliberately narrow — only the keyword-led shapes (`fn`,
`def`, `function`), and only when the line's last non-whitespace character is
the `(` that opens the parameter list. The generic `name(params) {` method shape
is NOT relaxed, because `name(` alone is exactly what a multi-line CALL looks
like; relaxing it would drop real call sites. `const x = quoteShipping(` is a
call, not a definition, because the binding names `x`.

**(b) Lenient accounting.** `account_site` replaces the exact `file:line`
lookup:

- a reviewer line accounts for a listed site when its symbol matches and its
  FILE matches, whatever line number it carries — or none at all
  (`CALLSITE quoteShipping | src/cart/cartSummary.ts | UPDATED | …`);
- a line number pointing at another line in the same file still accounts for
  that file's sites; one UPDATED/UNAFFECTED line for a file accounts for every
  listed site in it;
- an explicit MISSED for a line marks exactly that line; an explicit MISSED for
  a file (no line number) marks all of that file's sites;
- file paths match when equal or when one is a whole-COMPONENT suffix of the
  other (`admin/manualQuote.ts` ≡ `src/admin/manualQuote.ts`), so
  `src/a/quote.ts` and `src/b/quote.ts` are never confused;
- the per-symbol requirement is kept: a listed symbol with no CALLSITE line at
  all is unaccounted, even when another symbol in the same file was answered.

**(c) Outcomes unchanged.** `missing_call_sites` (unaccounted) and
`absent_call_sites` (no line at all) keep their meaning on top of the new
matcher, so the orchestrator's Lane R66a split is untouched: absent-only with no
other finding → `CallSiteTableIncomplete` → UNCERTAIN → one objective
escalation; an explicit MISSED → REQUEST_CHANGES → send-back.

The complaint text now names files and symbols. New
`unaccounted_call_site_files` groups the unaccounted sites one entry per
(symbol, FILE) with `complaint()` / `evidence()`;
`gate_decision_with_call_sites` emits ONE `CallSiteCompleteness` finding per
group:

> E7: no CALLSITE line accounted for quoteShipping in src/shipping/quote.ts —
> emit one CALLSITE line per changed symbol per file (`CALLSITE quoteShipping |
> src/shipping/quote.ts | UPDATED` or `UNAFFECTED`); the line number is optional
> and one line covers every site listed in that file

with evidence still the exact `file:line`s (unchanged for a single-site group,
so the existing gate and headless-path assertions hold). The R66a
incomplete-table reason likewise names `<symbol> in <file>` per file and says
one line per file covers a whole file.

**(e) The guarantee is intact.** The list itself is still complete and
recall-first (name-based matching, definitions excluded, nothing else dropped);
an explicit MISSED still requests changes; TEST_INTEGRITY and every other lens
are untouched; a genuinely missed caller in a file the reviewer never names is
still unaccounted and still holds the gate.

## The prompt change (`review_gate.rs::REVIEW_SYSTEM_PROMPT`)

The E7 lens now asks for the rule the harness enforces: one `CALLSITE` line per
changed symbol per FILE, the line number OPTIONAL, one line covering every site
listed for that file, and a `file:line` on a MISSED line when only one site in
the file is the problem. It also states that the list never contains the
symbol's own definition. The grammar line became
`CALLSITE <symbol> | <file>[:line] | UPDATED|UNAFFECTED|MISSED | <one-line note>`
and the closing paragraph matches. No other lens or rule was touched.

## Tests

`cost_router::call_site_completeness`:

- `r67a_a_reflowed_definition_site_is_never_listed_as_a_call_site` — the r67 s1
  repo snapshot with both reflowed definitions: `computeManualQuote` has zero
  call sites (definition-only file), `quoteShipping` has exactly its four real
  callers and not its definition head.
- `r67a_a_multiline_call_is_still_a_call_site` — a multi-line CALL, which also
  ends at an open paren, is not mistaken for a definition.
- `r67a_one_line_per_file_accounts_for_that_files_sites` — the r67 s1 list as
  recorded (six sites) with the Fable-style response: the four answered files
  are accounted; the two remaining FILES are named in the complaint, with their
  symbols and the one-line-per-file rule.
- `r67a_one_line_covers_several_sites_in_one_file_whatever_the_line_number` —
  no line number, one line number, a line number matching no listed site, and a
  shortened path all account for both sites in the file.
- `r67a_an_explicit_missed_still_marks_a_file_or_a_line` — a file-level MISSED
  marks both sites (and is not "absent"); a line-numbered MISSED marks only that
  line even when another line in the file was UPDATED.
- `r67a_a_symbol_with_no_callsite_line_at_all_is_unaccounted`.
- `r67a_a_same_named_file_in_another_directory_does_not_account`.
- `r67a_the_r67_s1_review_accounts_for_every_site_once_definitions_are_excluded`
  — (a)+(b) together: the same response that was sent back now accounts for
  every listed site.
- `r67a_a_site_ref_with_no_line_number_is_read_as_a_whole_file`.

`cost_router::review_gate`:

- `r67a_one_line_per_file_accounts_for_every_site_and_the_approve_stands`.
- `r67a_an_unaccounted_file_is_one_finding_naming_the_file_and_the_rule`.
- `system_prompt_names_e7_and_the_callsite_grammar` extended to pin the new
  grammar and the one-line-per-file / optional-line-number wording.

`agents::platform_extensions::orchestrator`:

- `r67a_one_callsite_line_per_file_lets_the_round_approve` — per-file lines make
  the round APPROVE; a file left out still reads as an incomplete table whose
  reason names `quoteShipping in src/shipping/quote.ts` and the rule.

Every pre-existing E7 / R66a / R65a / salvage / goose-server headless-path
assertion was kept green without modification (one finding per file collapses to
the old one-per-site shape whenever a file holds a single site).

## Suite results (M1, `1.97-aarch64-apple-darwin`)

```
cargo test -p permagent --lib cost_router::call_site_completeness
test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 5631 filtered out; finished in 0.02s

cargo test -p permagent --lib r66a_
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 5650 filtered out; finished in 0.61s

cargo test -p permagent --lib r65a_
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 5653 filtered out; finished in 0.10s

cargo test -p permagent --lib salvage_review
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 5662 filtered out; finished in 0.06s

cargo test -p permagent --lib dispatch_cross_family_reviewer
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5664 filtered out; finished in 0.16s

cargo test -p permagent --lib r67a_
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 5653 filtered out; finished in 0.01s

cargo test -p permagent --lib
test result: ok. 5658 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 232.03s

cargo clippy -p permagent --lib -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 6m 57s   (exit 0, no warnings)
```

## Risks

- **Leniency can hide a real miss in a file with several call sites.** One
  UPDATED line for a file now accounts for every listed site in it, so a
  reviewer that looked at one of two callers in a file and answered by file
  accounts for both. Mitigation: the line list is still handed to the reviewer
  in full (`file:line` per site), the reviewer can mark a specific line MISSED,
  and the worker's own verify step plus TEST_INTEGRITY remain. The alternative —
  the strict rule — demonstrably parks correct work, and a review it never
  reaches catches nothing at all.
- **Path-suffix matching.** `admin/manualQuote.ts` accounts for
  `src/admin/manualQuote.ts`. Whole-component comparison keeps distinct
  directories distinct, but a repo with `src/a/admin/manualQuote.ts` and
  `src/b/admin/manualQuote.ts` could cross-account on a shortened path; the
  reviewer is shown, and asked for, full paths.
- **Definition-head recognition is narrow by design.** A definition shape not in
  the keyword-led list (a class method whose parameter list is reflowed, say) is
  still listed as its own call site. That is the safe failure: the reviewer can
  dismiss it with one UNAFFECTED line, whereas widening the relaxation to bare
  `name(` would drop real call sites. Widen it per language as the loop shows
  one, never generically.
- **Enumerated call-site counts drop** (r67 s1: 6 → 4), which feeds
  `ReviewFacts::call_sites` and hence review-complexity/floor selection. The
  counts are now truthful — a definition was never a call site — but a diff
  sitting on a complexity boundary can route to a cheaper reviewer than before.
