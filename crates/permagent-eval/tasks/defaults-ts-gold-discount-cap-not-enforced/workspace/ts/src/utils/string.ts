export function truncate(input: string, maxLength: number): string {
  if (input.length <= maxLength) return input;
  return input.slice(0, maxLength) + "...";
}

export function capitalize(input: string): string {
  if (input.length === 0) return input;
  return input[0].toUpperCase() + input.slice(1);
}

export function slugify(input: string): string {
  return input
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)/g, "");
}
