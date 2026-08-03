import { describe, it, expect } from 'vitest';
import { extensionLabel, type ToolExtension } from './SettingsView';

/**
 * Regression: the Tools & MCPs panel crashed the whole app.
 *
 * `ExtensionEntry` flattens `ExtensionConfig` on the Rust side, and its
 * variants do not share a shape — `display_name` is an `Option<String>` that
 * the `stdio` variant omits from the payload entirely. The TypeScript type
 * declared it a required `string`, and the render did:
 *
 *     ext.display_name[0]?.toUpperCase()
 *
 * The optional chain is one character too late: `undefined[0]` throws before
 * `?.` is ever reached. React unmounted the tree and the window went blank.
 *
 * It fired for exactly two extensions — Brave Search and Tavily Web Search —
 * which are the two stdio servers, and the two the user opened the tab to
 * find. Those exact payloads are pinned below.
 */

const BRAVE: ToolExtension = {
  enabled: true,
  type: 'stdio',
  name: 'Brave Search',
  description: 'Web + local search via the Brave Search API.',
  bundled: null,
  available_tools: [],
  env_keys: ['BRAVE_API_KEY'],
  // display_name deliberately absent — this is the real wire shape.
};

const TAVILY: ToolExtension = {
  enabled: true,
  type: 'stdio',
  name: 'Tavily Web Search',
  description: 'AI-optimized web search and content extraction via Tavily.',
  bundled: null,
  available_tools: [],
  env_keys: ['TAVILY_API_KEY'],
};

describe('extensionLabel', () => {
  it('falls back to name for stdio servers that send no display_name', () => {
    expect(extensionLabel(BRAVE)).toBe('Brave Search');
    expect(extensionLabel(TAVILY)).toBe('Tavily Web Search');
  });

  it('never returns an empty string, so the avatar initial is always safe', () => {
    const cases: ToolExtension[] = [
      BRAVE,
      TAVILY,
      { enabled: false, type: 'stdio', name: '' },
      { enabled: false, type: 'stdio', name: '', display_name: '' },
      { enabled: false, type: 'stdio', name: '', display_name: null },
      { enabled: false, type: 'stdio', name: '   ', display_name: '   ' },
    ];
    for (const ext of cases) {
      const label = extensionLabel(ext);
      expect(label.length).toBeGreaterThan(0);
      // The exact expression the crash came from must now be safe.
      expect(() => label.charAt(0).toUpperCase()).not.toThrow();
    }
  });

  it('prefers display_name when it is present and meaningful', () => {
    expect(extensionLabel({
      enabled: true, type: 'builtin', name: 'developer', display_name: 'Developer',
    })).toBe('Developer');
  });

  it('ignores a whitespace-only display_name in favour of the name', () => {
    expect(extensionLabel({
      enabled: true, type: 'builtin', name: 'developer', display_name: '  ',
    })).toBe('developer');
  });

  it('tolerates a null display_name without throwing', () => {
    expect(() => extensionLabel({
      enabled: true, type: 'stdio', name: 'x', display_name: null,
    })).not.toThrow();
  });
});
