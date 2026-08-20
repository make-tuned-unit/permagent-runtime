/**
 * Grow Actions are grouped by the category the generator assigned — Measurement,
 * Acquisition, UX, and the rest — so the panel is a set of tabs rather than one
 * long tagged list. The keys are the allowlist `parse_actions` in
 * `growth_actions.rs` accepts; anything else falls through to UX, matching the
 * backend fallback, so a garbled category cannot invent a tab.
 */

export const ACTION_CATEGORY_ORDER = [
  'measurement',
  'acquisition',
  'conversion',
  'retention',
  'churn',
  'ux',
  'content',
  'seo',
  'aeo',
] as const;

export type ActionCategory = (typeof ACTION_CATEGORY_ORDER)[number];

export const ACTION_CATEGORY_LABELS: Record<ActionCategory, string> = {
  measurement: 'Measurement',
  acquisition: 'Acquisition',
  conversion: 'Conversion',
  retention: 'Retention',
  churn: 'Churn',
  ux: 'UX',
  content: 'Content',
  seo: 'SEO',
  aeo: 'AEO',
};

export function normalizeActionCategory(raw: string): ActionCategory {
  return (ACTION_CATEGORY_ORDER as readonly string[]).includes(raw)
    ? (raw as ActionCategory)
    : 'ux';
}

export interface CategoryGroup<T extends { category: string }> {
  key: ActionCategory;
  label: string;
  actions: T[];
}

/** Non-empty groups, in the stable order above — not in first-seen order, so a
 *  new review cannot reshuffle the tab strip. */
export function groupActionsByCategory<T extends { category: string }>(
  actions: T[],
): CategoryGroup<T>[] {
  const buckets = new Map<ActionCategory, T[]>();
  for (const action of actions) {
    const key = normalizeActionCategory(action.category);
    const list = buckets.get(key);
    if (list) list.push(action);
    else buckets.set(key, [action]);
  }
  return ACTION_CATEGORY_ORDER.filter((key) => buckets.has(key)).map((key) => ({
    key,
    label: ACTION_CATEGORY_LABELS[key],
    actions: buckets.get(key)!,
  }));
}
