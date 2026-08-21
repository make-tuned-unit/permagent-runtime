/**
 * Settings → Features — pure helpers for the switches pane. The five workers
 * here are OFF by default; each is a plain boolean config key the daemon
 * re-reads on every tick of its loop, so a flip lands at the next tick with no
 * restart. Kept free of React so the copy and the key table are unit-testable.
 *
 * These keys are ALSO written by the agent's own page under Settings → Agents,
 * and `strix_enabled` additionally by the Guard block in the Models pane. One
 * key, three places, never a second key — the daemon serialises the key it reads
 * on every agent row (`gate.config_key`), and every surface writes THAT through
 * the same `/config/upsert` call, so the surfaces cannot drift apart.
 *
 * The set mirrors the Rust gate table
 * (`crates/goose/src/agents/self_knowledge/mod.rs::worker_gate`), which pins it
 * from the other side.
 */

export type FeatureKey =
  | 'initiative_enabled'
  | 'playbook_enabled'
  | 'concierge_enabled'
  | 'steward_scan_enabled'
  | 'strix_enabled';

export type FeatureRow = {
  key: FeatureKey;
  label: string;
  /** One honest line: what it does, and where its output lands. */
  what: string;
  /** How soon a flip takes effect. Every loop here re-reads the flag per tick. */
  effect: string;
};

/** Row order is the display order. */
export const FEATURE_ROWS: readonly FeatureRow[] = [
  {
    key: 'initiative_enabled',
    label: 'Initiative',
    what: 'Watches your activity for a terminal command you keep repeating and, once you have gone quiet, proposes automating it on the Decision Inbox. It only ever proposes.',
    effect: 'Off by default. Takes effect at the next tick (about a minute), no restart.',
  },
  {
    key: 'playbook_enabled',
    label: 'Decision Playbook',
    what: 'Periodically distills your answered decisions and draft edits into a few provenance-linked hints about how you tend to decide, and recalls them when a roadmap is planned. Hints, never rules.',
    effect: 'Off by default. Takes effect at the next tick, no restart.',
  },
  {
    key: 'concierge_enabled',
    label: 'Concierge',
    what: 'Reads your Gmail inbox read-only on the local model, flags what needs you, and proposes an editable reply draft as a Decision-Inbox card. It can never send or change mail.',
    effect: 'Off by default. Takes effect at the next tick (up to a few hours), no restart.',
  },
  {
    key: 'steward_scan_enabled',
    label: 'Steward git-health',
    what: 'Sweeps one active project per pass for repo hygiene (stale branches, unpushed work, dirty trees) and files proposals only — every cleanup is a Decision-Inbox approval.',
    effect: 'Off by default. Takes effect within about 15 minutes, no restart.',
  },
  {
    key: 'strix_enabled',
    label: 'The Guard (security sweeps)',
    what: 'Sweeps ONE of your own projects per pass — rotating, least-recently-scanned first — for exposed secrets, vulnerable dependencies, injection and access-control weaknesses, and files a security report with a fix plan as a note on that project. It reports only: it never edits code to fix what it found. Needs the external `strix` scanner and Docker, locally or on the host in `strix_docker_ssh`, and each sweep spends your API credits.',
    effect: 'Off by default. Takes effect within about 15 minutes, no restart. Sweep cadence is a cost dial under Settings → Models.',
  },
];

export const FEATURE_KEYS: readonly FeatureKey[] = FEATURE_ROWS.map(r => r.key);

/** The exact CLI the Concierge precondition points at. */
export const GMAIL_CONNECT_COMMAND = 'permagent integrations connect gmail';

export type IntegrationStatus = { provider: string; connected: boolean; token_present: boolean };

/** True only when the daemon reports a stored Gmail token. `null` = unknown. */
export function gmailTokenPresent(list: IntegrationStatus[] | null): boolean | null {
  if (list === null) return null;
  const gmail = list.find(i => i.provider === 'gmail');
  return gmail ? gmail.token_present : false;
}

/**
 * The Concierge precondition line. The toggle stays live in every state — the
 * loop is inert without a token, so enabling early is harmless — but the copy
 * says plainly what is missing.
 */
export function conciergePreconditionCopy(tokenPresent: boolean | null): string {
  if (tokenPresent === true) return 'Gmail token present.';
  if (tokenPresent === false) return `Needs a Gmail token: run \`${GMAIL_CONNECT_COMMAND}\`. Until then the loop stays idle.`;
  return 'Checking for a Gmail token…';
}

/**
 * The daemon answers `/config/read` with the bare JSON value (`true`, `null`,
 * …). Only a literal `true` counts as on — a missing key, `"true"` or `1` is off,
 * exactly as the Rust `get_param::<bool>` would read it.
 */
export function readFlag(raw: unknown): boolean {
  return raw === true;
}
