# Coding-harness context packet contract

`crates/goose/src/context_packet.rs` is the observable projection for one
coding request. It is deliberately a projection only: durable memory remains
Spectral, and the packet never writes or maintains a parallel memory store.

Every packet emits these five slots in stable order:

1. fixed harness prompt;
2. tool schema;
3. project memory;
4. retrieved Spectral memory;
5. tool output.

Each slot carries `Present`/`Missing { reason }`, character count, a
deterministic `ceil(chars / 4)` token estimate, provenance, and whether the
slot is protected fixed contract material. Missing data is not represented as
zero tokens. Spectral memory keys and retrieval provenance references are
deduplicated first-seen, so repeated recall hits cannot inflate packet size or
hide which retrieval produced the context.

The projection is pure and has no provider, SQL, embedder, or network
dependency. The CLI's authenticated Brain bridge now emits a packet receipt on
the `permagent.context_packet` tracing target for successful recalls, including
the Spectral search-result provenance it has at that boundary; unavailable
fixed/tool/project/tool-output slots remain explicitly missing. Provider-
reported usage remains the authoritative post-call token count; these values
are pre-call estimates and must be labeled as such in reports.

## E2 graduation gaps

The packet contract is implemented and deterministically tested for fixed-slot
visibility, protected facts, missing-data semantics, token estimates, and
memory/provenance deduplication. The CLI recall seam is wired and tested, but
the full model-request assembler still needs to pass its real fixed prompt,
tool schema, project memory, and tool-output values into the same packet and
join the receipt with provider usage for a benchmark run. Until that wiring
exists, context-packet coverage is `Unrated`, not `Excellent`.
