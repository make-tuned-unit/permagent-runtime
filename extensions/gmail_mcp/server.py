"""Gmail MCP extension — stdio server for permagentd."""

import json
from typing import Any

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

from . import gmail_client

app = Server("gmail-mcp")


@app.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="gmail__search",
            description="Search emails using Gmail query syntax (same as Gmail search bar)",
            inputSchema={
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Gmail search query"},
                    "max_results": {"type": "integer", "description": "Max results to return", "default": 10},
                },
                "required": ["query"],
            },
        ),
        Tool(
            name="gmail__read",
            description="Read full email content by message ID",
            inputSchema={
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "description": "Gmail message ID"},
                },
                "required": ["message_id"],
            },
        ),
        Tool(
            name="gmail__list_labels",
            description="List all Gmail labels/folders",
            inputSchema={
                "type": "object",
                "properties": {},
            },
        ),
        Tool(
            name="gmail__list_threads",
            description="List recent email threads",
            inputSchema={
                "type": "object",
                "properties": {
                    "max_results": {"type": "integer", "description": "Max threads to return", "default": 20},
                    "query": {"type": "string", "description": "Optional Gmail search query", "default": ""},
                },
            },
        ),
        Tool(
            name="gmail__send",
            description="Send an email (requires gmail.send scope)",
            inputSchema={
                "type": "object",
                "properties": {
                    "to": {"type": "string", "description": "Recipient email address"},
                    "subject": {"type": "string", "description": "Email subject"},
                    "body": {"type": "string", "description": "Plain text email body"},
                },
                "required": ["to", "subject", "body"],
            },
        ),
    ]


@app.call_tool()
async def call_tool(name: str, arguments: dict[str, Any]) -> list[TextContent]:
    try:
        if name == "gmail__search":
            result = gmail_client.search(
                query=arguments["query"],
                max_results=arguments.get("max_results", 10),
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
        elif name == "gmail__send":
            result = gmail_client.send(
                to=arguments["to"],
                subject=arguments["subject"],
                body=arguments["body"],
            )
        else:
            return [TextContent(type="text", text=f"Unknown tool: {name}")]
        return [TextContent(type="text", text=json.dumps(result, indent=2))]
    except Exception as e:
        return [TextContent(type="text", text=f"Error: {e}")]


async def serve():
    """Run the MCP server over stdio."""
    async with stdio_server() as (read_stream, write_stream):
        await app.run(read_stream, write_stream, app.create_initialization_options())


def main():
    import asyncio
    asyncio.run(serve())


if __name__ == "__main__":
    main()
