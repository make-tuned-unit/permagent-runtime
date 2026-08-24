/**
 * Claim-once id sets — the shape a "this must happen exactly once" guard takes
 * when the thing being guarded is a side effect (opening a terminal tab) and
 * the trigger can be delivered more than once.
 *
 * Deliberately module-scope-friendly rather than a React ref: a ref is exactly
 * what a remount forgets, and forgetting is the bug this guards against.
 */

export interface ClaimSet {
  /** True when THIS call owns the id; false when it was already claimed. */
  claim(id: string): boolean;
  /** Test-only reset. */
  reset(): void;
}

/** Bounded FIFO of claimed ids — an unbounded set in a long-lived session
 *  (Build stays mounted for the app's whole life) is a slow leak. Set
 *  preserves insertion order, so the first value is the oldest. */
export function createClaimSet(limit: number): ClaimSet {
  const seen = new Set<string>();
  return {
    claim(id: string): boolean {
      if (seen.has(id)) return false;
      seen.add(id);
      if (seen.size > limit) {
        const oldest = seen.values().next().value;
        if (oldest !== undefined) seen.delete(oldest);
      }
      return true;
    },
    reset(): void {
      seen.clear();
    },
  };
}
