"""Quick smoke test: verify MCP server starts and lists 4 tools.

Run: python3 test_mcp_init.py
"""
import asyncio

from permagent_gmail_mcp.server import list_tools


async def test():
    tools = await list_tools()
    print(f"Tools registered: {len(tools)}")
    for t in tools:
        print(f"  - {t.name}: {t.description[:60]}...")

    expected = {"gmail__search", "gmail__read", "gmail__list_labels", "gmail__list_threads"}
    actual = {t.name for t in tools}
    assert actual == expected, f"Mismatch: got {actual}, expected {expected}"
    print("\nAll 4 tools registered correctly.")


if __name__ == "__main__":
    asyncio.run(test())
