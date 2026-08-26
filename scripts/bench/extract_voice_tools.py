#!/usr/bin/env python3
"""Extract ONLY the tool schemas from a recorded daemon LLM request.

The voice bench needs the exact tool surface a voice turn carries (~124 schemas,
~39k tokens). Those schemas are static, machine-generated MCP declarations — they
contain no conversation text, no memory and no personal context. Everything else
in the recorded request (the system prompt's live-status tail, the messages) IS
personal, so this script reads the first record of a
`~/.permagent/logs/llm_request.*.jsonl` file and writes out `input.tools` and
nothing else.

The output is deliberately NOT checked in: regenerate it locally before a bench
run.

    python3 scripts/bench/extract_voice_tools.py \
        ~/.permagent/logs/llm_request.3.jsonl /tmp/voice_bench_tools.json
"""

import json
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    src, dst = sys.argv[1], sys.argv[2]

    tools = None
    with open(src, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            candidate = record.get("input", {}).get("tools")
            if candidate:
                tools = candidate
                break

    if not tools:
        print(f"no request in {src} carried a tools array", file=sys.stderr)
        return 1

    # Two normalisations, both to hand the bench the same `rmcp::model::Tool`
    # the agent would have built:
    #   * drop the `cache_control` marker the Anthropic formatter stamps onto the
    #     last schema — the bench re-derives breakpoints through the repo's own
    #     formatter, so a leftover marker would double up
    #   * rename `input_schema` back to `inputSchema`; the log records the
    #     Anthropic wire spelling, `Tool` uses the MCP one
    cleaned = []
    for tool in tools:
        entry = {k: v for k, v in tool.items() if k != "cache_control"}
        if "input_schema" in entry:
            entry["inputSchema"] = entry.pop("input_schema")
        cleaned.append(entry)

    with open(dst, "w", encoding="utf-8") as fh:
        json.dump(cleaned, fh)

    print(f"wrote {len(cleaned)} tool schemas to {dst}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
