#!/usr/bin/env python3
"""Chat bench tables."""
import json, statistics, sys, collections

ORDER = ["haiku", "dschat", "kimi25", "gpt54mini", "glm53", "sonnet5"]


def pct(xs, p):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    k = max(0, min(len(xs) - 1, int(round((p / 100) * (len(xs) - 1)))))
    return xs[k]


def main(path):
    d = json.load(open(path))
    rows = d["results"]
    cands = [c for c in ORDER if any(r["candidate"] == c for r in rows)]
    print(f"# spent ${d['spent']:.2f}; stopped: {d.get('stopped')}\n")
    print("| candidate | n | TTFT med | TTFT p90 | first sentence med | total med | "
          "startup | tool ok | silent | thinks | $/turn (non-tool) | cache read |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for c in cands:
        rs = [r for r in rows if r["candidate"] == c]
        meta = d["per_candidate"].get(c, {})
        led = meta.get("ledger") or {}
        ttfts = [r["ttft_ms"] for r in rs if r["ttft_ms"] is not None]
        tool_rows = [r for r in rs if r["tool_correct"] is not None]
        ok = sum(1 for r in tool_rows if r["tool_correct"])
        silent = sum(1 for r in rs if r["silent_tool_call"])
        thinks = sum(1 for r in rs if r["thought"])
        calls = led.get("calls") or 0
        inp, cr = led.get("input") or 0, led.get("cache_read") or 0
        print(f"| {c} | {len(rs)} | {pct(ttfts,50)} | **{pct(ttfts,90)}** | "
              f"{pct([r['first_sentence_ms'] for r in rs],50)} | "
              f"{pct([r['total_ms'] for r in rs],50)} | {meta.get('startup_ms')} | "
              f"{ok}/{len(tool_rows)} | {silent}/{len(rs)} | {thinks}/{len(rs)} | "
              f"${(led.get('usd') or 0)/calls:.4f} | {cr/inp*100 if inp else 0:.0f}% |")
    print("\nAll times in ms. TTFT INCLUDES the CLI startup shown in its own column; "
          "the daemon's /reply pays that once per session, not per turn.\n")

    print("## by turn kind (TTFT median)\n")
    kinds = ["conversational", "tool", "reasoning"]
    print("| candidate | " + " | ".join(kinds) + " |")
    print("|---|" + "---:|" * len(kinds))
    for c in cands:
        cells = []
        for k in kinds:
            v = pct([r["ttft_ms"] for r in rows
                     if r["candidate"] == c and r["kind"] == k], 50)
            cells.append(str(v) if v is not None else "—")
        print(f"| {c} | " + " | ".join(cells) + " |")

    print("\n## tool turns: what was asked for vs what was called\n")
    for c in cands:
        wrong = [(r["id"], r["tools"]) for r in rows
                 if r["candidate"] == c and r["tool_correct"] is False]
        print(f"- **{c}**: {len(wrong)} wrong — " +
              (", ".join(f"{i}:{','.join(t) or 'none'}" for i, t in wrong[:6]) or "—"))


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(f"usage: {sys.argv[0]} <results.json>")
    main(sys.argv[1])
