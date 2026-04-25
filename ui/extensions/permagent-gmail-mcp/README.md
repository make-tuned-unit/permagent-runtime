# permagent-gmail-mcp

Read-only Gmail MCP extension for Permagent. Exposes Gmail tools to the agent via the MCP stdio transport.

## Tools (Phase 1 — read-only)

| Tool | Description |
|------|-------------|
| `gmail__search` | Search emails using Gmail query syntax |
| `gmail__read` | Read full email content by message ID |
| `gmail__list_labels` | List all Gmail labels/folders |
| `gmail__list_threads` | List recent email threads with pagination |

## Prerequisites

**Phase 1 uses user-provided Google OAuth credentials.** A Permagent-owned OAuth project is planned for production.

1. Go to [Google Cloud Console](https://console.cloud.google.com/apis/credentials)
2. Create a new OAuth 2.0 Client ID (Desktop app type)
3. Enable the Gmail API for your project
4. Note your Client ID and Client Secret

## Installation

```bash
pip install -e ui/extensions/permagent-gmail-mcp/
```

## Connect Gmail

### Via CLI
```bash
permagent integrations connect gmail
```
This opens the browser for OAuth consent, exchanges the code for tokens, and stores them at `~/.permagent/secrets/gmail_token.json` (chmod 600).

### Via REST API (Command Center)
```bash
# Initiate OAuth flow
curl -X POST http://localhost:3000/integrations/gmail/connect \
  -H 'Content-Type: application/json' \
  -d '{"client_id": "...", "client_secret": "..."}'

# Revoke
curl -X DELETE http://localhost:3000/integrations/gmail
```

## Config

The extension is registered in `~/.permagent/config.yaml` after connecting:

```yaml
extensions:
  gmail:
    type: stdio
    cmd: permagent-gmail-mcp
    enabled: true
    envs:
      GMAIL_TOKEN_PATH: ~/.permagent/secrets/gmail_token.json
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `GMAIL_TOKEN_PATH` | `~/.permagent/secrets/gmail_token.json` | Path to OAuth token file |

## TODOs

- [ ] Permagent-owned Google OAuth project (no user-provided credentials)
- [ ] macOS Keychain storage for tokens (preferred over file)
- [ ] Write capabilities (gmail.send scope) in Phase 2
- [ ] Pagination cursors for large result sets
- [ ] Attachment download support
