"""Gmail MCP extension — read-only stdio server for permagentd (Phase 1)."""

import json
from typing import Any

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

from . import gmail_client

app = Server("permagent-gmail-mcp")


@app.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="gmail__search",
            description="Search emails using Gmail query syntax (same as Gmail search bar). "
            "Examples: 'from:alice subject:invoice', 'is:unread after:2026/04/01'",
            inputSchema={
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Gmail search query"},
                    "max_results": {
                        "type": "integer",
                        "description": "Max results (default 10, max 100)",
                        "default": 10,
                    },
                },
                "required": ["query"],
            },
        ),
        Tool(
            name="gmail__read",
            description="Read full email content by message ID. Returns headers, body text, and labels.",
            inputSchema={
                "type": "object",
                "properties": {
                    "message_id": {
                        "type": "string",
                        "description": "Gmail message ID (from search or thread listing)",
                    },
                },
                "required": ["message_id"],
            },
        ),
        Tool(
            name="gmail__list_labels",
            description="List all Gmail labels/folders (INBOX, SENT, custom labels, etc.)",
            inputSchema={
                "type": "object",
                "properties": {},
            },
        ),
        Tool(
            name="gmail__list_threads",
            description="List recent email threads with message counts. Supports optional search query.",
            inputSchema={
                "type": "object",
                "properties": {
                    "max_results": {
                        "type": "integer",
                        "description": "Max threads to return (default 20)",
                        "default": 20,
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional Gmail search query to filter threads",
                        "default": "",
                    },
                },
            },
        ),
    ]


@app.call_tool()
async def call_tool(name: str, arguments: dict[str, Any]) -> list[TextContent]:
    try:
        if name == "gmail__search":
            result = gmail_client.search(
                query=arguments["query"],
                max_results=min(arguments.get("max_results", 10), 100),
            )
        elif name == "gmail__read":
            result = gmail_client.read(message_id=arguments["message_id"])
        elif name == "gmail__list_labels":
            result = gmail_client.list_labels()
        elif name == "gmail__list_threads":
            result = gmail_client.list_threads(
                max_results=arguments.get("max_results", 20),
                query=arguments.get("query", ""),
            )
        else:
            return [TextContent(type="text", text=f"Unknown tool: {name}")]
        return [TextContent(type="text", text=json.dumps(result, indent=2))]
    except FileNotFoundError as e:
        return [TextContent(type="text", text=f"Gmail not connected: {e}")]
    except Exception as e:
        return [TextContent(type="text", text=f"Gmail error: {e}")]


async def serve():
    """Run the MCP server over stdio."""
    async with stdio_server() as (read_stream, write_stream):
        await app.run(read_stream, write_stream, app.create_initialization_options())


def main():
    import asyncio
    asyncio.run(serve())


if __name__ == "__main__":
    main()
