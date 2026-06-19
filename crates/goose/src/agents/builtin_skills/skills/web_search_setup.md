---
name: web-search-setup
description: Guide the user through enabling web search by connecting the Brave and/or Tavily MCP providers. Use when the user asks to set up, enable, or turn on web search, or wants you to be able to search the web and you currently cannot. Walk them through getting an API key and pasting it into the credential manager.
---

Use this skill when the user wants you to be able to search the web and no
search tool is currently available to you (no `brave_search` / `tavily` tools).

Web search ships as two optional MCP providers — **Brave Search** and **Tavily**.
Both have free tiers. The user adds a key for either (or both); you then choose
per query. You cannot add the key for them — they must create it on the
provider's site and paste it into the app. Your job is to make that painless.

## Provider key pages
- **Brave Search:** https://api-dashboard.search.brave.com/app/keys
- **Tavily:** https://app.tavily.com/

## Steps

1. **Offer and ask which provider.** Briefly explain both have free tiers and
   ask whether they want Brave, Tavily, or both. If they don't care, suggest
   starting with one.

2. **Post the key-page link as a normal Markdown link.** Write the provider's
   key-page URL (above) as a clickable link in your reply. When the user clicks
   it, it opens in the in-app browser on the Build tab — it does NOT leave the
   app. Tell them to click it.

3. **Read the page and guide them.** After they click, call
   `read_browser_content` to see what's on screen, then guide them step by step
   from where they actually are — signing in / signing up, finding the API-keys
   area, creating a key, and copying it. Re-read the page with
   `read_browser_content` as they move, so your guidance matches the live page.
   Do not guess what the page shows — read it.

4. **Land them at the credential field.** Tell them to open
   **Settings → API keys → "Search & tools"**, paste the key into the matching
   provider's field, and press Save. Saving stores the key in the OS keychain
   and turns that provider on automatically.

5. **Confirm and offer the second provider.** Once saved, the search tool
   becomes available to you. Confirm it's working with a quick search if they
   like, then offer to repeat for the other provider if they wanted both.

## Notes
- Keys are stored encrypted in the system keychain and never leave the device.
- Until a key is saved and the provider enabled, no web request is made — web
  search is off by default and the user opts in by completing this setup.
- If both providers are connected, prefer alternating between them across
  queries so the user stays within each free tier.
