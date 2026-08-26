#!/usr/bin/env python3
"""Turn the harness sweep JSON into the report's tables."""
import json, statistics, sys, collections

PRICES = {  # USD per 1M tokens, from the repo's canonical table / published_prices.rs
    "deepseek-v4-flash":         dict(inp=0.44, out=1.32, cr=0.014),
    "deepseek-chat":             dict(inp=0.28, out=0.42, cr=0.028),
    "claude-haiku-4-5-20251001": dict(inp=1.00, out=5.00, cr=0.10),
    "claude-haiku-4.5":          dict(inp=1.00, out=5.00, cr=0.10),
    "claude-sonnet-5":           dict(inp=3.00, out=15.00, cr=0.30),
    "glm-5.3":                   dict(inp=1.40, out=4.40, cr=0.26),
    "glm-4.7":                   dict(inp=0.60, out=2.20, cr=0.11),
    "kimi-k2.5":                 dict(inp=0.60, out=3.00, cr=0.10),
    "gpt-5.4-mini":              dict(inp=0.75, out=4.50, cr=0.075),
    "gpt-5.4-mini-2026-03-17":   dict(inp=0.75, out=4.50, cr=0.075),
    "kimi-k2.5":                 dict(inp=0.60, out=3.00, cr=0.10),
}
ORDER = ["dschat", "haiku", "kimi25", "gpt54mini", "sonnet5", "glm53"]
# What each candidate is BILLED as. Two of these are aliases, which is why the
# tables name the billed model rather than the id we asked for.
BILLED_AS = {
    "dschat": "deepseek-v4-flash", "haiku": "claude-haiku-4-5-20251001",
    "kimi25": "kimi-k2.5", "gpt54mini": "gpt-5.4-mini-2026-03-17",
    "sonnet5": "claude-sonnet-5", "glm53": "glm-5.3",
}


def split_spend(row):
    """(own, leaked): spend on the model under test vs on anything else.

    The harness's `delegate`/`summon` path does not honour the PERMAGENT_PACK_*
    pins, so a run can quietly bill a THIRD model. Folding that into the
    candidate's number would price one model's work as another's.
    """
    own = leaked = 0.0
    want = BILLED_AS[row["candidate"]]
    for b in row.get("ledger_billed") or []:
        (own if b["model"] == want else leaked).__class__  # noqa: B018
        if b["model"] == want:
            own += b["usd"] or 0
        else:
            leaked += b["usd"] or 0
    return own, leaked


def corrected(row, share):
    """Cost if the prompt cache worked at `share`, at the billed model's rates."""
    want = BILLED_AS[row["candidate"]]
    mine = [b for b in (row.get("ledger_billed") or []) if b["model"] == want]
    p = PRICES.get(want)
    if not p or not mine:
        return None
    i = sum(b.get("input") or 0 for b in mine)
    o = sum(b.get("output") or 0 for b in mine)
    return ((1 - share) * i * p["inp"] + share * i * p["cr"] + o * p["out"]) / 1e6


def main(path):
    d = json.load(open(path))
    rows = d["results"]
    tasks = list(dict.fromkeys(r["task"] for r in rows))
    cands = [c for c in ORDER if any(r["candidate"] == c for r in rows)]
    complete = [t for t in tasks
                if len([r for r in rows if r["task"] == t]) == len(cands)]

    # The cache share Anthropic actually achieved here — the projection's basis.
    anth = [r for r in rows if r["candidate"] in ("haiku", "sonnet5") and r.get("ledger_input")]
    share = (sum(r["ledger_cache_read"] or 0 for r in anth) /
             sum(r["ledger_input"] for r in anth)) if anth else 0.0
    print(f"# complete rows: {len(complete)} of {len(tasks)}; "
          f"measured Anthropic cache-read share {share:.0%}; spent ${d['spent']:.2f}")
    print(f"# stopped: {d.get('stopped')}\n")

    print("| candidate | billed as | solved | $ measured | $/solved | "
          "$ cache-corrected | leaked $ | med s | med calls | med tools | input tok | output tok | cache read |")
    print("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for c in cands:
        rs = [r for r in rows if r["candidate"] == c and r["task"] in complete]
        if not rs:
            continue
        solved = sum(1 for r in rs if r["solved"])
        cost = sum(split_spend(r)[0] for r in rs)
        leaked = sum(split_spend(r)[1] for r in rs)
        corr = [corrected(r, share) for r in rs]
        corr_sum = sum(x for x in corr if x is not None) if all(x is not None for x in corr) else None
        billed = BILLED_AS[c]
        mine = [b for r in rs for b in (r.get("ledger_billed") or [])
                if b["model"] == BILLED_AS[c]]
        cr = sum(b.get("cache_read") or 0 for b in mine)
        inp = sum(b.get("input") or 0 for b in mine)
        outp = sum(b.get("output") or 0 for b in mine)
        tmo = sum(1 for r in rs if r.get("timed_out"))
        print(f"| {c} | `{billed}` | {solved}/{len(rs)} | ${cost:.2f} | "
              f"{('$%.2f' % (cost/solved)) if solved else '—'} | "
              f"{('$%.2f' % corr_sum) if corr_sum is not None else '—'} | "
              f"{('$%.2f' % leaked) if leaked else '—'} | "
              f"{statistics.median(r['secs'] for r in rs):.0f}{'*' if tmo else ''} | "
              f"{statistics.median(r['ledger_rows'] for r in rs):.0f} | "
              f"{statistics.median(r['tool_calls'] for r in rs):.0f} | "
              f"{inp/1000:.0f}k | {outp/1000:.0f}k | {cr/inp*100 if inp else 0:.0f}% |")

    print("\n## per task\n")
    print("| task | " + " | ".join(cands) + " |")
    print("|---|" + "---|" * len(cands))
    for t in tasks:
        cells = []
        for c in cands:
            r = next((x for x in rows if x["task"] == t and x["candidate"] == c), None)
            cells.append("—" if not r else
                         ("PASS" if r["solved"] else "fail") +
                         f" {r['secs']:.0f}s ${split_spend(r)[0]:.2f}")
        print(f"| {t.replace('defaults-','')} | " + " | ".join(cells) + " |")

    print("\n## tools used (median per run)\n")
    for c in cands:
        rs = [r for r in rows if r["candidate"] == c and r["task"] in complete]
        names = collections.Counter(n for r in rs for n in r.get("tool_names") or [])
        print(f"- **{c}**: " + ", ".join(f"{k} {v}" for k, v in names.most_common(8)))


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(f"usage: {sys.argv[0]} <results.json>")
    main(sys.argv[1])
