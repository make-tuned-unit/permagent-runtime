/**
 * Turning a provider's raw model ids into something a person can choose from.
 *
 * Two separate problems, both reported by Jesse as "I can't find the latest
 * Haiku model" and "the dropdown picker for models is ugly".
 *
 * The first is not cosmetic. The picker asked the daemon for each provider's
 * live model list and REPLACED the provider's own known-models list with the
 * answer. That live list is filtered through the bundled canonical registry
 * (`fetch_recommended_models` → `filter_map`), so any id the registry has not
 * heard of is dropped silently — even when the provider ships it as a known
 * model and the cost router happily routes to it. A model can therefore be
 * fully supported and completely unselectable. `mergeModelLists` fixes that by
 * taking the union: the live list orders the menu, the known list guarantees
 * nothing the app already supports can go missing.
 *
 * The second is that `claude-haiku-4-5-20251001` in a flat mono list is not a
 * name, it is a serial number. `describeModel` recovers the family, the
 * version and the release date that are already encoded in the id, so the menu
 * can read "Claude Haiku 4.5" with the id underneath and sort newest first.
 */

export type ModelEntry = {
  /** The id to send to the daemon. Never shown as the primary label. */
  id: string;
  /** "Claude Haiku 4.5" — what the person is actually looking for. */
  label: string;
  /** "Anthropic" family grouping within a provider, or null. */
  family: string | null;
  /** Sort key: higher is newer/more capable. */
  rank: number;
  /** YYYYMMDD from a dated id, for the secondary line. */
  released: string | null;
  /** True when this id is a dated pin of another entry in the same list. */
  dated: boolean;
};

/**
 * Union of the live list and the provider's known list.
 *
 * Order follows the live list, because the daemon sorts it by release date
 * (newest first) and that is the order a person expects. Anything known but
 * not live is appended rather than dropped — it is supported, it just did not
 * survive the registry filter, and hiding it is how a model becomes
 * "impossible to find".
 */
export function mergeModelLists(live: readonly string[], known: readonly string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const id of [...live, ...known]) {
    const trimmed = id.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    out.push(trimmed);
  }
  return out;
}

const FAMILY_WORDS = ['opus', 'sonnet', 'haiku', 'fable'] as const;

/** Capitalise a token for display: "haiku" → "Haiku", "gpt" → "GPT". */
function titleCase(word: string): string {
  if (/^(gpt|llm|ai|xl|vl)$/i.test(word)) return word.toUpperCase();
  return word.charAt(0).toUpperCase() + word.slice(1);
}

/**
 * Read an id into a display name, a family and a sort rank.
 *
 * This is pattern-matching on ids, not a lookup table, deliberately: a table
 * would have to be edited every time a model ships, and a model missing from
 * the table is exactly the failure being fixed here. An id that matches
 * nothing still gets a usable label — its own id — and sorts last rather than
 * disappearing.
 */
export function describeModel(id: string): ModelEntry {
  const lower = id.toLowerCase();

  // A trailing 8-digit date is a pinned release: claude-haiku-4-5-20251001.
  const dateMatch = lower.match(/[-@](\d{8})$/);
  const released = dateMatch ? dateMatch[1] : null;
  const base = released ? lower.slice(0, dateMatch!.index) : lower;

  const family = FAMILY_WORDS.find(word => base.includes(word)) ?? null;

  // Version digits: "claude-haiku-4-5" → 4.5, "claude-sonnet-5" → 5,
  // "gpt-4o" → 4. Hyphenated pairs are a decimal, which is the Anthropic form.
  let version: number | null = null;
  const pair = base.match(/(\d+)-(\d+)(?!\d)/);
  if (pair) {
    version = Number(`${pair[1]}.${pair[2]}`);
  } else {
    const single = base.match(/(\d+(?:\.\d+)?)/);
    if (single) version = Number(single[1]);
  }

  let label: string;
  if (family && version !== null) {
    const vendor = base.startsWith('claude') ? 'Claude' : titleCase(base.split(/[-/]/)[0]);
    label = `${vendor} ${titleCase(family)} ${version}`;
  } else {
    // Unknown shape: show the id, tidied, rather than inventing a name.
    label = id;
  }

  // Rank: version dominates, family breaks ties so the capable line sorts
  // above the fast one within a generation, and a dated pin sits just under
  // the undated alias it pins so the two read as one entry.
  const familyWeight = family ? { fable: 3, opus: 2, sonnet: 1, haiku: 0 }[family] ?? 0 : 0;
  const rank = (version ?? -1) * 10 + familyWeight - (released ? 0.5 : 0);

  return { id, label, family, rank, released, dated: released !== null };
}

/** Describe and sort a provider's models: newest and most capable first. */
export function catalogue(ids: readonly string[]): ModelEntry[] {
  return ids
    .map(describeModel)
    .sort((a, b) => b.rank - a.rank || a.id.localeCompare(b.id));
}

/**
 * Does this entry match what the person typed?
 *
 * Matching the label as well as the id is the point: somebody hunting for
 * "haiku 4.5" should not have to know it is spelled `claude-haiku-4-5`. The
 * query is matched with its separators stripped so "haiku45", "haiku 4.5" and
 * "haiku-4-5" are all the same search.
 */
export function matchesQuery(entry: ModelEntry, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const loose = (text: string) => text.toLowerCase().replace(/[-_.\s@]/g, '');
  return (
    entry.id.toLowerCase().includes(q)
    || entry.label.toLowerCase().includes(q)
    || loose(entry.id).includes(loose(q))
    || loose(entry.label).includes(loose(q))
  );
}
