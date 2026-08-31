/**
 * Finance labs — opt-in Polybot / Picker, a user-owned ticker universe, and
 * how Picker rows sort against the Financier's close judgment.
 *
 * Keys match the daemon (`polybot_enabled`, `picker_enabled`, `picker_universe`).
 * Missing is off: the cards do not appear until the user turns them on.
 */

export const POLYBOT_ENABLED_KEY = 'polybot_enabled';
export const PICKER_ENABLED_KEY = 'picker_enabled';
export const PICKER_UNIVERSE_KEY = 'picker_universe';
export const FUNDAMENTALS_KEY = 'FINANCIAL_DATASETS_API_KEY';

/** How many pick rows show before "Show the rest". Tall cards were the problem. */
export const PICKS_PREVIEW = 6;

export const MAX_UNIVERSE = 80;

export const POLYBOT_DISCLAIMER = [
  'Polybot can place real orders on Polymarket with money in the wallet those keys control.',
  'You can lose the entire bankroll. Permagent does not give investment advice, does not size or guarantee trades, and does not reimburse losses.',
  'Keys stay in the macOS keychain on this device. You are solely responsible for the bot, its strategy, and any orders it places.',
  'Pause stops new orders. Turning the card off does not cancel open positions on Polymarket.',
].join(' ');

export const PICKER_DISCLAIMER =
  'Picker ranks tickers you list. It does not place brokerage orders. A pick is a hypothesis, not advice to buy. The Financier may flag one name for you to review. You decide whether to trade anywhere else.';

/** Split a pasted universe into unique uppercase tickers. Mirrors picker::parse_universe. */
export function parseUniverse(raw: string): string[] {
  const out: string[] = [];
  for (const token of raw.split(/[,;\s]+/)) {
    const t = token.trim().replace(/^\$/, '').toUpperCase();
    if (!isTicker(t)) continue;
    if (out.includes(t)) continue;
    out.push(t);
    if (out.length >= MAX_UNIVERSE) break;
  }
  return out;
}

function isTicker(s: string): boolean {
  if (s.length === 0 || s.length > 12) return false;
  if (!/^[A-Z0-9.-]+$/.test(s)) return false;
  return /[A-Z]/.test(s);
}

export function formatUniverse(tickers: string[]): string {
  return tickers.join('\n');
}

/** Append pasted tokens onto an existing extras list. Caps at MAX_UNIVERSE. */
export function appendUniverse(existing: string[], raw: string): string[] {
  const next = [...existing];
  for (const t of parseUniverse(raw)) {
    if (next.includes(t)) continue;
    next.push(t);
    if (next.length >= MAX_UNIVERSE) break;
  }
  return next;
}

/**
 * Plain-language summary of why the significance check dropped a name.
 *
 * The daemon sends whole sentences ("in-sample ICIR 0.12 is below 0.3 — likely
 * noise"). A tag has room for three words, and "loop kill" — what the tag used
 * to say — explains nothing to anyone who has not read `pick_loop.rs`. The full
 * sentences stay in the tooltip and the expanded row; this is the headline.
 */
export function killSummary(kills: string[]): string {
  const first = kills.find((k) => k.trim().length > 0);
  if (!first) return 'filtered out';
  const k = first.toLowerCase();
  if (k.includes('not enough')) return 'too little history';
  if (k.includes('could not')) return 'not measurable';
  if (k.includes('noise')) return 'looks like noise';
  if (k.includes('half-life')) return 'fades too fast';
  if (k.includes('overfit') || k.includes('out-of-sample')) return 'did not hold up';
  if (k.includes('bonferroni')) return 'too many names tested';
  return 'filtered out';
}

/** What the red/green tag on a pick row actually says. */
export function loopTagLabel(loop: { passed: boolean; kills: string[] } | null | undefined): string {
  if (!loop) return '';
  return loop.passed ? 'signal checked' : `filtered: ${killSummary(loop.kills)}`;
}

export function pickIsApproved(ticker: string, approved: string | null | undefined): boolean {
  if (!approved) return false;
  return ticker.trim().toUpperCase() === approved.trim().toUpperCase();
}

export type PickSortable = {
  ticker: string;
  loop?: { passed: boolean } | null;
  rank?: number | null;
};

/** Financier-approved first, then loop pass, then rank. */
export function sortPicks<T extends PickSortable>(
  picks: T[],
  approvedTicker: string | null | undefined,
): T[] {
  return [...picks].sort((a, b) => {
    const aOk = pickIsApproved(a.ticker, approvedTicker) ? 0 : 1;
    const bOk = pickIsApproved(b.ticker, approvedTicker) ? 0 : 1;
    if (aOk !== bOk) return aOk - bOk;
    const aPass = a.loop?.passed ? 0 : 1;
    const bPass = b.loop?.passed ? 0 : 1;
    if (aPass !== bPass) return aPass - bPass;
    const ar = a.rank ?? 999;
    const br = b.rank ?? 999;
    return ar - br;
  });
}

export function requiredKeysSet(fields: Array<{ key: string; required: boolean }>, masked: Record<string, string>): {
  have: number;
  need: number;
} {
  const need = fields.filter((f) => f.required).length;
  const have = fields.filter((f) => f.required && Boolean(masked[f.key])).length;
  return { have, need };
}
