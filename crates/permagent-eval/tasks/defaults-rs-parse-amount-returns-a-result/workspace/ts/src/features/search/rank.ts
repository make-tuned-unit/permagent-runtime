import type { SearchItem } from "./filter.ts";

export function rankResults(items: SearchItem[]): SearchItem[] {
  return [...items].sort((a, b) => b.score - a.score);
}
