/**
 * Canonical accessor for the type discriminator of a daemon `/events` message.
 *
 * The wire names this field inconsistently by construction: the top-level
 * envelope (`PermagentEvent`) tags its type as `type` (serde `rename = "type"`),
 * while the inner `ActivityEvent` uses `event_type`. Both carry snake_case
 * values, and no single message ever sets both fields — so resolving `type`
 * first, then `event_type`, is correct for every message shape.
 *
 * Reading only one field silently misses the other (a handler that never
 * fires). Always route `/events` reads through this helper so a future
 * consumer can't get the field name wrong.
 */
export function wireEventType(msg: unknown): string {
  if (msg && typeof msg === 'object') {
    const m = msg as { type?: unknown; event_type?: unknown };
    if (typeof m.type === 'string') return m.type;
    if (typeof m.event_type === 'string') return m.event_type;
  }
  return '';
}
