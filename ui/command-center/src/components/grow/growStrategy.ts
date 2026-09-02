/**
 * The five-pillar GTM strategy spine and the brand kit: prompt seeds, the
 * tolerant reads of `metadata_json`, and the two saves.
 *
 * Split out of GrowView.tsx (R9). Pure data and fetch helpers, no JSX — the
 * cards that render them live in StrategyLens.tsx.
 */

import { apiFetch } from '../../lib/api';
import type { Project } from '../projects/types';

// Appended to every Grow prompt that DRAFTS user-facing copy (value props,
// posts, outreach) so the output reads like a sharp human wrote it, not a
// chatbot. The full voice spec lives in the "humanize" builtin skill; this
// names it and inlines the top AI tells so the draft is humanized even before
// the skill loads. Strategy prompts (audience/positioning/channels) deliberately
// omit it — they produce internal analysis, not copy the user will publish.
export const HUMANIZE_VOICE =
  ' Write it the way a sharp person actually writes: lead with the point, stay specific and concrete, keep sentences short, and cut every AI tell (no em-dashes, no hype words like "seamless" or "leverage" or "unlock", no throat-clearing openers). Apply your "humanize" skill for the full voice spec before you hand it back.';

// The five GTM pillars (research: target market · value prop · pricing &
// positioning · channels · integrated marketing) — the strategy spine every
// launch needs. Each is a Henry-assisted prompt seed.
export const PILLARS: { key: string; label: string; prompt: (p: string) => string; hint: string }[] = [
  {
    key: 'audience',
    label: 'Audience',
    hint: 'Who is this for, and where do they already gather?',
    prompt: (p) => `For the project "${p}", define the target audience: the specific people who need this, their watering holes (subreddits, communities, hashtags), and the one persona to lead with. Use what you know from the project's Brain, people, and docs.`,
  },
  {
    key: 'value',
    label: 'Value proposition',
    hint: 'The one sentence that makes them care.',
    prompt: (p) => `Draft 3 one-line value propositions for "${p}" — the sharp promise that makes the target audience stop scrolling. Ground them in the project's actual capabilities.${HUMANIZE_VOICE}`,
  },
  {
    key: 'positioning',
    label: 'Positioning & price',
    hint: 'Against what, and for how much?',
    prompt: (p) => `For "${p}", propose positioning against the 2-3 real alternatives people use today, and a pricing hypothesis (free/paid tiers) that fits the audience.`,
  },
  {
    key: 'channels',
    label: 'Channels',
    hint: 'The 2-3 places to show up, not all of them.',
    prompt: (p) => `Recommend the 2-3 highest-leverage launch channels for "${p}" (e.g. a specific subreddit, X, a newsletter, a directory) and why each fits this audience — not a generic list.`,
  },
  {
    key: 'workback',
    label: 'Workback schedule',
    hint: 'Milestones counting back from launch day.',
    prompt: (p) => `Build a workback schedule for "${p}" from its launch date: the dated milestones between now and launch, working backwards.`,
  },
  {
    key: 'content',
    label: 'Content & launch',
    hint: 'The hub piece and the posts that orbit it.',
    prompt: (p) => `For "${p}", outline the launch content: one substantial hub piece (a guide/thread that establishes authority) and a week of social posts that link back to it. Draft the first post so I can schedule it.${HUMANIZE_VOICE}`,
  },
];

// ── Saved strategy (metadata_json.strategy — #13) ────────────────────────────
export interface SavedPillar {
  content: string;
  updated_at?: string;
  /** Labeled bullets [{label, detail}] — rendered as the card's rich body. */
  points?: Array<{ label: string; detail: string }>;
  /** Stat chips [{label, value}] — rendered as a metric row. */
  metrics?: Array<{ label: string; value: string }>;
}

/** Tolerant read of a saved pillar from the project's metadata bag. */
export function readStrategy(project: Project, key: string): SavedPillar | null {
  const strategy = (project.metadataJson as { strategy?: Record<string, unknown> } | null)?.strategy;
  const raw = strategy?.[key] as { content?: unknown; updated_at?: unknown } | undefined;
  if (!raw || typeof raw.content !== 'string' || !raw.content.trim()) return null;
  const pairs = (v: unknown, a: string, b: string) =>
    Array.isArray(v)
      ? (v as Array<Record<string, unknown>>)
          .filter(item => typeof item?.[a] === 'string' && typeof item?.[b] === 'string')
          .map(item => ({ [a]: item[a] as string, [b]: item[b] as string }))
      : undefined;
  const rawAny = raw as Record<string, unknown>;
  return {
    content: raw.content,
    updated_at: typeof raw.updated_at === 'string' ? raw.updated_at : undefined,
    points: pairs(rawAny.points, 'label', 'detail') as SavedPillar['points'],
    metrics: pairs(rawAny.metrics, 'label', 'value') as SavedPillar['metrics'],
  };
}

export async function saveStrategy(projectId: string, pillar: string, content: string): Promise<void> {
  await apiFetch(`/api/projects/${encodeURIComponent(projectId)}/strategy/${encodeURIComponent(pillar)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content }),
  });
}

/** Run-all: one turn where Henry produces and SAVES every pillar. */
export function runAllPrompt(projectName: string): string {
  return `Build the complete go-to-market strategy for "${projectName}" using everything you know about the project (Brain, people, docs, goals). Work through all five pillars — audience, value, positioning, channels, content — and for EACH one, save your result with the set_project_strategy tool (project: "${projectName}", pillar: "<key>"): content = a 2-3 sentence summary, points = [{label, detail}] labeled specifics (personas with watering holes, channels with fit reasons, alternatives with your counter-positioning), metrics = [{label, value}] stat chips (price hypothesis, audience size, post cadence). The Strategy cards render this as rich content, so fill all three fields. Also save the "workback" pillar: the launch workback schedule — points = [{label: "<date or week>", detail: "<milestone>"}] counting back from launch day. THEN save this project's brand with set_project_brand (voice, origin story of why it was built, palette from its real product if you know it, donts). THEN turn the workback into real to-dos: create a Kanban card on this project's board for each concrete milestone with the card_create tool (title = the milestone, description = why it matters and its target week). Finish with a one-paragraph summary.${HUMANIZE_VOICE}`;
}

export function draftPostPrompt(projectName: string): string {
  return `For "${projectName}", call social_content_brief first and draft from THAT project's brief only — a top-performing page, a newly completed goal/feature, or the saved origin story. Create it as a social_post with card_create: title = the hook, description = the post body, post_status = "draft", harvest_kind set, format and channel that fit. Omit scheduled_for so the daemon picks the send time. A still matching this post generates automatically; do not set scheduled yourself.${HUMANIZE_VOICE}`;
}

export function brandPrompt(projectName: string): string {
  return `For "${projectName}", save this project's brand kit with set_project_brand: voice (how it writes), origin (why it was built, quoted from this project), bg/fg/accent as #RRGGBB from its real site or product UI if you know them, and donts for generated media. Use only this project — do not copy another project's kit.`;
}

export interface ProjectBrand {
  voice: string;
  origin: string;
  bg: string;
  fg: string;
  accent: string;
}

export function readBrand(project: Project): ProjectBrand {
  const raw = (project.metadataJson as { brand?: Record<string, unknown> } | null)?.brand;
  const s = (k: string) => (typeof raw?.[k] === 'string' ? (raw[k] as string) : '');
  return { voice: s('voice'), origin: s('origin'), bg: s('bg'), fg: s('fg'), accent: s('accent') };
}

export async function saveBrand(projectId: string, brand: ProjectBrand): Promise<void> {
  await apiFetch(`/api/projects/${encodeURIComponent(projectId)}/brand`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(brand),
  });
}
