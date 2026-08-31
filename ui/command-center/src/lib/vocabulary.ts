/**
 * `vocabulary` — one user-facing word per concept, and one gloss per piece of
 * jargon, imported by every surface that renders it.
 *
 * The audit found the same thing named two ways on one screen more than once:
 * a scheduled job was a "recipe" in the section header and an "automation" in
 * the button that creates it; a memory's weight was "reinforcement" in the
 * Brain's graph panel and "signal" in its list, on the same number, in the same
 * tab. Neither was a typo — each surface was written on its own and picked a
 * word on its own, which is exactly what a shared module prevents. Two
 * components rendering the same number now import the same label.
 *
 * The glosses are the second half of the same rule: a term the interface
 * invents must be defined where it is first used, once, in the app's own voice
 * — not left as chrome the user is expected to already understand.
 *
 * Keep this file boring. It holds words, never markup and never logic.
 */

export interface Term {
  /** Singular, mid-sentence. */
  one: string;
  /** Plural, mid-sentence. */
  many: string;
  /** Title case, for a header or a button. */
  title: string;
  /** One sentence saying what it is, in the user's terms. */
  gloss: string;
}

/**
 * A scheduled job. The tab is Automate, the create button says Create
 * Automation, the modal is New Automation and the delete says "Delete
 * automation" — only the section header and its empty states still said
 * "recipe", which is the internal type's name (`ScheduledJob` / `RecipeCard`),
 * not the user's.
 */
export const AUTOMATION: Term = {
  one: 'automation',
  many: 'automations',
  title: 'Automations',
  gloss: 'A saved instruction the agent runs on a schedule, or on demand.',
};

/**
 * A learned procedure. Named consistently already — it is here so the Library's
 * front door and Automate's condensed list keep agreeing as both change.
 */
export const SKILL: Term = {
  one: 'skill',
  many: 'skills',
  title: 'Skills',
  gloss: 'A procedure the agent learned from your work and can re-run without being re-taught.',
};

/**
 * How strongly a memory is held. Rendered as "reinforcement" in the Brain's
 * graph panel and "signal" in its list — two words, one number, one tab. The
 * ruled word is "strength", which neither surface had to invent and which says
 * what the percentage means without a metaphor.
 */
export const MEMORY_STRENGTH: Term = {
  one: 'strength',
  many: 'strength',
  title: 'Strength',
  gloss:
    'How strongly this memory is held — it grows each time the memory proves useful and '
    + 'decays while it goes unused.',
};

/**
 * Glosses for terms the interface borrows from a domain rather than inventing.
 * Each is one sentence, stated where the term first appears.
 */
export const GLOSSARY = {
  /** Finance — Picker's loop metrics. */
  icir:
    'ICIR — how consistently this loop\'s picks beat the market, relative to how much they '
    + 'swing. Higher is steadier; below zero means the loop has been wrong more than right.',
  halfLife:
    'Half-life — how long one of this loop\'s edges keeps working before it decays to half its '
    + 'strength. A short half-life means the loop has to be re-run often to stay useful.',
  /** Finance — the Financier's approval mark on a pick. */
  financierApproved:
    'The Financier reviewed this pick and approved it: the numbers behind it held up to a '
    + 'second look.',
  /** Grow — the two estimates on an action card. */
  impactConfidence:
    'Impact is how much this could move the metric if it works; confidence is how sure the '
    + 'model is that it will. Both are the model\'s own estimate at the time it wrote the card, '
    + 'not a measurement.',
  /** Projects — the earned-privilege ladder on the verification panel. */
  cleanRuns:
    'A clean run is one where verification finished with nothing to fix. Enough of them in a '
    + 'row and this project earns wider access for the agent, one level at a time.',
  /** Build — the cost statusline's three compressed segments. */
  costMeter:
    'Tokens sent and received this session, what prompt caching saved you, and how much of the '
    + 'model\'s context window is in use. "incl. N subagents" means the total already counts '
    + 'work done by agents this one dispatched.',
} as const;

export type GlossaryKey = keyof typeof GLOSSARY;

/** "1 automation" / "4 automations" — the count and the right form of the word. */
export function plural(term: Term, n: number): string {
  return `${n} ${n === 1 ? term.one : term.many}`;
}
