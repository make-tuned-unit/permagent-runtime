# Gmail MCP Extension

MCP stdio server providing Gmail tools for the Permagent daemon.

## Tools

| Tool | Description |
|------|-------------|
| `gmail__search` | Search emails by query (Gmail search syntax) |
| `gmail__read` | Read full email by message ID |
| `gmail__list_labels` | List all labels/folders |
| `gmail__list_threads` | List recent threads with pagination |
| `gmail__send` | Send an email (requires `gmail.send` scope) |

## Setup

1. Install dependencies: `pip install -r requirements.txt`
2. Connect Gmail via Command Center Settings or CLI: `permagent integrations connect gmail`
3. Token is stored at `~/.permagent/secrets/gmail_token.json`

## Running

As a standalone MCP server (stdio):
```bash
python -m extensions.gmail_mcp.server
```

Or via permagentd extension config in `~/.permagent/config.yaml`:
```yaml
extensions:
  gmail:
    type: stdio
    cmd: python
    args: ["-m", "extensions.gmail_mcp.server"]
    envs:
      GMAIL_TOKEN_PATH: ~/.permagent/secrets/gmail_token.json
    enabled: true
    timeout: 30
```
