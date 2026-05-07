# Agent Capability Audit

## Summary
- **Capabilities audited:** 8 (A through H)
- **Available and enabled:** 3 (file reading, shell execution, file writing)
- **Available but disabled:** 0
- **Available via shell workaround:** 3 (web search, web fetching, system inventory)
- **Not available as dedicated tool:** 2 (browser automation, process control as structured tools)
- **Audit date:** 2026-05-07

The agent's tool surface is centered on the **developer extension** (write, edit, shell, tree) and the **analyze extension** (tree-sitter AST). There are **no built-in web search, web fetch, browser automation, or system inventory tools**. However, the `shell` tool can execute `curl`, `du`, `df`, `system_profiler`, and other CLI tools — making most capabilities available indirectly through shell commands.

---

## A. File System Reading

**Status:** Available and enabled

### Tools

**`tree` tool** (developer extension)
- Lists directory contents recursively with line counts
- Respects .gitignore, .gitexclude, .ignore
- Parameters: `path` (required), `depth` (optional, default 2, 0 = unlimited)
- Returns text tree format with line counts per file

**`shell` tool** — for metadata and hashing
- `ls -la` for file metadata (size, modified date, permissions)
- `find` for recursive scanning with filters
- `shasum` / `md5` for file hashing
- `stat` for detailed file info

**`analyze` tool** (analyze extension)
- Tree-sitter AST parsing for code structure
- Parameters: `path`, `focus`, `max_depth` (default 3), `follow_depth` (default 2), `force`
- Returns function/class/import counts, call graphs

### File content reading
The agent reads file contents via the `shell` tool (`cat`, `head`, `tail`) or by using the `edit` tool's error path (which shows file preview on match failure). There is no dedicated `read_file` tool — reading is done through shell commands.

### Skip patterns
The `tree` tool respects .gitignore automatically. For shell commands, the agent must manually exclude paths (e.g., `find . -not -path './.git/*'`). There is no built-in skip list for sensitive paths like `~/.ssh` or `.env` — the agent's judgment and the permission system are the guardrails.

### Limitations
- No dedicated `read_file` tool — relies on `shell` + `cat`
- No built-in duplicate detection (hashing requires shell commands)
- `tree` respects gitignore but doesn't support custom exclude patterns beyond what gitignore provides
- Large file reading limited by shell output cap (2000 lines / 50KB per stream)

---

## B. Shell Command Execution

**Status:** Available and enabled

### Tool: `shell`

**Parameters:**
- `command` (string, required) — shell command to execute
- `timeout_secs` (u64, optional) — timeout in seconds (0 or omitted = no timeout)

**Return shape:**
```json
{
  "stdout": "string (up to 2000 lines)",
  "stderr": "string (up to 2000 lines)",
  "exit_code": "i32 or null (null if killed)",
  "timed_out": "bool",
  "output_truncated": "bool",
  "output_collection_error": "string or null"
}
```

**Shell resolution:**
- macOS/Linux: Uses `bash` (falls back to `sh`), respects `GOOSE_SHELL` env var
- Resolves login shell PATH from profile script (cached at startup)
- Flatpak sandboxing: wraps commands with `flatpak-spawn --host` when detected

### Sandboxing / Scoping
- **No directory scoping.** The agent can execute commands in any directory.
- **No command blocklist.** Any command available in the shell PATH can be executed.
- **Permission system is the guardrail.** GooseMode (Smart Approve, Manual, etc.) controls whether shell commands require user approval. In automated/scheduled execution, the default is to allow.
- **Output limits:** 2000 lines per stream (stdout/stderr), 50KB byte limit. Overflow saved to temp file with path provided.
- **Background process handling:** 500ms drain timeout after exit — backgrounded processes that continue producing output get truncated.

### Configuration required
None — works out of the box on macOS.

---

## C. Web Search

**Status:** Not available as a dedicated tool. Available via shell workaround.

### What exists
No built-in `web_search` tool. No integration with Google, Bing, DuckDuckGo, SerpAPI, Tavily, or any search provider.

### Shell workaround
The agent can use the `shell` tool to run:
```bash
curl -s "https://html.duckduckgo.com/html/?q=query+here" | grep -o '<a[^>]*href="[^"]*"[^>]*>' | head -10
```
Or use `lynx -dump`, `w3m`, or similar text-mode browsers if installed.

### What would be needed to add it
**Small-medium work.** Options:
1. **MCP server:** Install a web search MCP server (e.g., `@anthropic/mcp-server-web-search` or a DuckDuckGo scraper). This is the cleanest approach — add an extension config pointing to the MCP server binary. No core code changes needed.
2. **Built-in tool:** Add a `web_search` tool to the developer extension or a new extension. Medium work (~200 lines). Would need an API key for any commercial search API.
3. **Shell-based recipe instruction:** Tell the agent in the recipe prompt to use `curl` + DuckDuckGo HTML endpoint. No code changes but fragile and slow.

### API key requirements
- DuckDuckGo HTML: No API key (scraping, no rate limit guarantee)
- Google Custom Search: API key + Custom Search Engine ID
- Tavily: API key ($5/month for 1000 searches)
- SerpAPI: API key (100 free searches/month)

---

## D. Web Fetching

**Status:** Not available as a dedicated tool. Available via shell workaround.

### What exists
No built-in `web_fetch` or `fetch_url` tool.

### Shell workaround
The agent can use:
```bash
curl -s "https://example.com" | head -200
```
This works for static HTML. Does not render JavaScript.

### JavaScript-rendered pages
Not supported via shell. Would require a headless browser (Puppeteer, Playwright) or a rendering service.

### Authentication
The agent can pass headers via `curl -H "Authorization: Bearer ..."` but has no cookie jar or session management.

### What would be needed
**Small work.** Same options as web search:
1. MCP server with fetch capability (many exist)
2. Built-in tool wrapping `reqwest` (~100 lines)
3. Recipe instruction to use `curl` (works for most cases)

---

## E. Browser Automation

**Status:** Not available

### What exists
The desktop shell has an embedded browser (WKWebView overlay in `ui/desktop/src-tauri/src/browser.rs`), but this is a **UI surface**, not an agent-accessible tool. The agent cannot programmatically open URLs, click elements, or extract text from the embedded browser.

Goose ships a bundled MCP server called `computer-controller` (referenced in CLI at `permagent mcp computer-controller`), but this is a **separate MCP server binary** that would need to be installed and configured as an extension. It's not enabled by default.

### Shell workaround
Limited options:
- `open "https://url"` opens in system default browser (macOS) — no control after that
- `osascript` can automate Safari (if enabled in Safari Accessibility settings)
- No headless browser available by default

### What would be needed
**Significant work** for full browser automation. Options:
1. Enable `computer-controller` MCP server as a default extension
2. Add Playwright/Puppeteer-based tool (requires Node.js dependency)
3. Build a Tauri command that the agent can invoke to control the embedded browser (new capability)

---

## F. System Inventory

**Status:** Not available as a dedicated tool. Fully available via shell.

### Shell-based system inventory (all work on macOS today)

| Information | Command | Notes |
|-------------|---------|-------|
| Installed apps | `ls /Applications/ && ls ~/Applications/` | Lists .app bundles |
| App details | `system_profiler SPApplicationsDataType -json` | Full app inventory with versions |
| Disk usage | `df -h` | Filesystem usage |
| Directory sizes | `du -sh ~/Documents ~/Downloads ~/Desktop` | Per-directory breakdown |
| Large files | `find ~ -size +100M -type f 2>/dev/null` | Files over threshold |
| Memory | `vm_stat` or `sysctl hw.memsize` | Physical + virtual |
| Processes | `ps aux --sort=-%mem \| head -20` | Top memory consumers |
| Battery | `pmset -g batt` | Battery status + health |
| Uptime | `uptime` | System uptime and load |
| CPU | `sysctl -n machdep.cpu.brand_string` | CPU model |
| macOS version | `sw_vers` | OS version info |
| Disk health | `diskutil info /` | Volume info |

### What would be needed for a dedicated tool
**Not needed.** Shell commands cover all system inventory needs on macOS. A dedicated tool would only add convenience (structured JSON output instead of parsing CLI output). Low priority.

---

## G. File System Writing

**Status:** Available and enabled

### Tools

**`write` tool** (developer extension)
- Creates new files or overwrites existing files
- Automatically creates parent directories
- Parameters: `path` (required), `content` (required)
- No confirmation prompt (permission system is the guardrail)

**`edit` tool** (developer extension)
- Surgical text find-and-replace
- Parameters: `path`, `before` (exact match), `after` (replacement)
- Fails gracefully on no match or multiple matches with helpful suggestions

**`shell` tool** — for move, rename, mkdir, delete
- `mv src dst` — move/rename files
- `mkdir -p path` — create directories
- `rm file` — delete files (permanent, not Trash)
- `cp src dst` — copy files

### Guardrails
- **No move-to-Trash.** `rm` is permanent deletion. There is no `trash` command built in. The agent could use `mv file ~/.Trash/` but this is not the same as macOS Trash (no `.DS_Store` metadata, no undo).
- **No blocked paths.** The agent can write to any path the daemon process has permission to access.
- **No scope restrictions.** No working-directory jail. The agent can write outside the project directory.
- **Permission system.** GooseMode controls whether file writes require approval. In Smart Approve mode, writes to the project directory are auto-approved; writes outside may trigger confirmation.
- **In scheduled execution:** Permission checks may auto-approve depending on configuration. This is the primary risk area for automated recipes.

### Limitations
- No atomic write (write-then-rename pattern)
- No file locking
- Overwrites without backup

---

## H. Process Control

**Status:** Available via shell. No dedicated tool.

### Shell-based process control (macOS)

| Action | Command | Notes |
|--------|---------|-------|
| Quit app | `osascript -e 'quit app "AppName"'` | Graceful quit via AppleScript |
| Force quit | `kill PID` or `killall AppName` | Hard termination |
| Restart app | `killall AppName && sleep 1 && open -a AppName` | Kill + reopen |
| Empty Trash | `osascript -e 'tell app "Finder" to empty the trash'` | Requires Finder |
| List processes | `ps aux` | Full process list |
| Process by name | `pgrep -l AppName` | Find PID |

### Guardrails
- AppleScript requires Accessibility permissions for some operations
- `kill` and `killall` work for user-owned processes only
- No sandboxing — the agent can kill any user process
- Permission system is the only guardrail

---

## Cross-Cutting Observations

### The shell tool is the universal adapter
Nearly every capability gap (web search, web fetch, system inventory, process control) can be filled by the `shell` tool running the appropriate CLI command. This makes the agent surprisingly capable even without dedicated tools — but the output is unstructured text that the LLM must parse, which is less reliable than structured tool returns.

### No web tools is the biggest gap
For starter Recipes that need web data (Daily Briefing, Weather, Tech Pulse), the agent must either:
1. Use `curl` + HTML parsing (fragile, no JS rendering)
2. Have a web search MCP server installed (cleanest, but requires setup)
3. Use the Anthropic API's built-in web search (if available via provider features)

### Permission system in automated context
When recipes run via the scheduler, there's no user present to approve tool calls. The permission system's behavior in unattended mode needs clarification — does it auto-approve everything, block dangerous operations, or queue for later approval?

### File writing without Trash is risky for automation
Automated recipes that clean up files (Storage Insights, Device Optimization) would permanently delete files unless the recipe explicitly uses `mv ~/.Trash/`. A "move to Trash" tool wrapper would be a safety improvement.

### MCP servers as the extension path
The cleanest way to add web search, web fetch, and browser automation is via MCP server extensions. Goose already has the extension manager infrastructure — adding an MCP server is a configuration change, not a code change. The question is which MCP servers to bundle as defaults.

---

## Recommendations for Starter Recipes

### Daily Briefing
**Requires:** C (web search), D (web fetching)
**Status:** Needs minor capability work

The agent can use `curl` to fetch web content, but reliable web search requires either:
- A DuckDuckGo HTML scraping approach in the recipe prompt (fragile but works)
- A web search MCP server (cleanest)

**Judgment: Needs minor capability work** — ship with `curl`-based web fetching in recipe prompt instructions, add web search MCP server as a fast-follow.

### Workspace Snapshot
**Requires:** A (file reading), F (system inventory)
**Status:** Ship-ready

All capabilities available via `tree`, `shell` (`du -sh`, `find`, `wc -l`), and `analyze`. No web or external dependencies.

**Judgment: Ship-ready**

### Storage Insights
**Requires:** A (file reading), B (shell), F (system inventory)
**Status:** Ship-ready

All capabilities available via `shell` (`du -sh`, `find -size`, `df -h`, `ls -la`). Disk analysis is entirely local.

**Judgment: Ship-ready** — but consider adding "move to Trash" safety wrapper for any cleanup suggestions.

### Weather Week Ahead
**Requires:** C (web search), D (web fetching)
**Status:** Needs minor capability work

Can use `curl` to fetch weather APIs (e.g., wttr.in, Open-Meteo) which return JSON. No API key needed for these free services.

**Judgment: Ship-ready with recipe-level workaround** — use `curl https://wttr.in/CityName?format=j1` or Open-Meteo API in recipe instructions.

### Device Optimization Report
**Requires:** A (file reading), B (shell), F (system inventory)
**Status:** Ship-ready

All capabilities available via `shell` (`system_profiler`, `df -h`, `du -sh`, `ps aux`, `pmset -g batt`, `vm_stat`). Entirely local.

**Judgment: Ship-ready**

### Tech Pulse
**Requires:** C (web search), D (web fetching)
**Status:** Needs minor capability work

Similar to Daily Briefing — needs web content. Can use `curl` to fetch RSS feeds or known tech news sites. Hacker News API (`https://hacker-news.firebaseio.com/v0/topstories.json`) is free and returns structured JSON.

**Judgment: Ship-ready with recipe-level workaround** — use public APIs (HN, Reddit JSON, RSS feeds) via `curl` in recipe instructions.

### Summary Table

| Recipe | Capabilities Required | All Available? | Judgment |
|--------|----------------------|----------------|----------|
| **Workspace Snapshot** | A, F | Yes | Ship-ready |
| **Storage Insights** | A, B, F | Yes | Ship-ready |
| **Device Optimization Report** | A, B, F | Yes | Ship-ready |
| **Weather Week Ahead** | C, D | Via curl + free APIs | Ship-ready (with recipe workaround) |
| **Tech Pulse** | C, D | Via curl + public APIs | Ship-ready (with recipe workaround) |
| **Daily Briefing** | C, D | Via curl (fragile for search) | Needs minor capability work |

**Bottom line:** 5 of 6 starter recipes can ship with current capabilities. The Daily Briefing is the only one that needs a proper web search solution, and even that can ship with a `curl`-based workaround using specific news site APIs rather than general web search.
