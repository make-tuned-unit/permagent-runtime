# UX implementation DAG

The executable inventory is [dag.json](dag.json). Each node contains dependencies, acceptance checks, status and evidence. `verified` requires acceptance evidence, not just source changes. D01 retains the earlier Home/Projects baseline while actual daemon wiring is checked.

```mermaid
flowchart TD
 D00[Research] --> D01[Home / Projects foundation]
 D00 --> D02[Loading / unread state]
 D01 --> D03[Watcher]
 D01 --> D04[Projects collection / board]
 D02 --> D04
 D02 --> D05[People]
 D02 --> D06[Automate]
 D01 --> D07[Grow]
 D02 --> D07
 D02 --> D08[Finance]
 D02 --> D09[Build / History]
 D05 --> D10[Brain / World]
 D02 --> D10
 D02 --> D11[Settings]
 D03 --> D12[Communication / navigation]
 D02 --> D12
 D03 & D04 & D05 & D06 & D07 & D08 & D09 & D10 & D11 & D12 --> D13[Full regressions / build / visual checks]
 D13 --> D14[Isolated end-to-end wiring]
```

Tests must exercise outcomes: retained drafts, exact navigation targets, no unintended writes, stale-response rejection, truthful error states and preservation of unrelated records. Source-string assertions alone do not verify a working route. Existing live data is never a mutable test fixture.


Run `python3 reports/ux-dag/check_dag.py` to validate dependency and evidence integrity. Add `--complete` for a gate that fails until every node is verified. Implementation and verification status are separate in the inventory. See [verification.md](verification.md) for page contracts, test commands, recorded results, rendered review and remaining boundaries.
