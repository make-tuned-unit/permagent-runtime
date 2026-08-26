export interface SearchItem {
  id: string;
  title: string;
  score: number;
  pinned?: boolean;
}

export function filterByQuery(items: SearchItem[], query: string): SearchItem[] {
  const q = query.trim().toLowerCase();
  if (q === "") return items;
  return items.filter((item) => item.title.toLowerCase().includes(q));
}
