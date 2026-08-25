export function formatCents(cents: number): string {
  const dollars = cents / 100;
  return `$${dollars.toFixed(2)}`;
}

export function formatDate(timestampMs: number): string {
  const d = new Date(timestampMs);
  return d.toISOString().slice(0, 10);
}
