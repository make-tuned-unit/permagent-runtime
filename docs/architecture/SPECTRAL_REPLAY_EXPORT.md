# Spectral replay export

Permagent exports its production recognition instrumentation as a versioned,
read-only replay bundle:

```sh
permagent export spectral-replay --out ./spectral-replay
permagent export spectral-replay --out ./spectral-replay --since 2026-07-23T00:00:00Z
permagent export spectral-replay --out ./spectral-replay --redact-queries
```

`--since` is inclusive and accepts an RFC 3339 timestamp. The command opens
`~/.permagent/spectral/permagent.db` with SQLite's read-only and query-only
flags. The daemon must have booted once with the additive recognition schema
repair before an older database can be exported.

## Bundle contract

The output directory contains `recognition.jsonl`, `recall.jsonl`, and
`manifest.json`. JSONL files contain one JSON object per physical line. Format
version 1 uses SHA-256 over the exact UTF-8 query bytes for `probe_hash`.

`recognition.jsonl` has exactly one row per recognition event, ordered by event
timestamp and retrieval id. Returned candidates are not duplicated here; their
complete set is carried by the corresponding `recall.jsonl` row.

```json
{"ts":"2026-07-30T12:34:56.789Z","event":"recognition","verdict":null,"familiarity":null,"memory_id":null,"probe_hash":"sha256-hex","outcome":null,"outcome_ts":"2026-07-30T12:35:10.123Z","retrieval_id":"retrieval-id"}
```

`memory_id` is reserved for the recognized trace from Spectral's `Recognized`
verdict variant; it is never sourced from recall candidates. It is NULL today,
along with `verdict` and `familiarity`, because `recognize()` is not wired.

`recall.jsonl` has one row per recall event:

```json
{"ts":"2026-07-30T12:34:56.789Z","event":"recall","query":"raw user text","probe_hash":"sha256-hex","returned":["memory-a","memory-b"],"ranks":[0,1],"scores":[0.91,0.84],"injected":["memory-a","memory-b"],"injected_source":"recorded","used":["memory-a"],"citation_checked_at":"2026-07-30T12:35:10.123Z","retrieval_id":"retrieval-id"}
```

`returned`, `ranks`, and `scores` are parallel arrays in ascending persisted
rank order. `returned` is the complete cascade result, while `injected` is the
post-filter set actually put into the system prompt (score at least 0.7, top
three). `injected_source` is `recorded` for forward rows captured at injection
time and `reconstructed` for historical rows deterministically replayed from
the persisted rank and signal score. An empty injected set is represented as
`[]`, not `null`.

`used` is `recognition_events.cited_memory_ids`. `used: null` means citation use
was never measured, while `used: []` means it was measured and no citation was
found. `citation_checked_at` is non-null when the turn-end detector ran,
including when it found no match, and null when use was never measured. With
`--redact-queries`, the `query` key is omitted entirely; `probe_hash` remains
available for joining the two feeds and detecting repeated probes.

The redacted export retains an unsalted SHA-256 of the query. That stable hash
is a dictionary-attackable pseudonym for short natural-language queries, not a
confidentiality guarantee; it is retained intentionally as a cross-export join
key for this local hand-off.

The manifest records source schema version, generation time, applied filters,
date range, source/export row counts, outcome-label counts, injection
provenance counts, the number of citation-checked events, and field-level NULL
status. In particular, it tells consumers whether every verdict and
familiarity value is NULL because the producing path is not wired.

## Signal semantics

- `useful`: a positive existing outcome was observed and at least one injected
  memory passed turn-end citation detection.
- `ignored`: recall returned at least one member, citation detection found no
  injected memory in the reply, and the existing outcome resolved positively.
- `wrong`: an existing negative signal fired, currently a decision bounce or
  an initiative observation bounce. No negative label is inferred from answer
  quality or model behavior.

Outcome and citation writes can arrive in either order. Each write derives the
coarse label from the signals present at that time, so an initially `ignored`
positive row becomes `useful` if its detached citation write lands afterward.
All writes are best-effort and detached from the reply path; failures are
logged and never fail a user turn.

## Citation detection

At recall time Permagent retains the content of only the injected top-K
memories in an in-memory turn handle. After the assistant stream ends, a
detached task normalizes punctuation and case and marks a memory used only when
the reply shares an exact contiguous five-word sequence of at least 24
characters with that memory. There is no LLM call and no semantic guess.

This deliberately favors precision over recall. Paraphrases will be false
negatives. The main false-positive mode is distinctive-looking boilerplate or
a five-word phrase independently repeated in both the memory and reply. The
detector never compares against the user query, and memory ids are not exposed
to the model, which avoids counting query repetition or id echoing as use.

## Historical limitations

Historical production outcomes are single-polarity: positive task resolutions
or no outcome. Verdict and familiarity are NULL because Spectral's
`recognize()` result is not wired to a production caller yet. Historical
`cited_memory_ids` values of `[]` do not prove that historical memories were
ignored: that column had no writer, so `citation_checked_at` is NULL and every
historical `outcome` remains NULL. The boot-time repair never derives a label
from an unmeasured empty citation set.

Historical injected sets are reconstructed exactly by replaying the injection
filter over the persisted members: signal score at least 0.7, ascending
persisted rank, limited to three. Both inputs are stored for every member, and
the filter constants predate the exported production window. Forward rows use
the directly recorded set instead. Today the honest `wrong` sources are
expected to remain near zero volume while the orchestrator/decision bounce
path is unused.
