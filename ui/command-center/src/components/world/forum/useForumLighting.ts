import { useCallback, useState } from 'react';

export type ForumLightingPreference = 'system' | 'day' | 'night';
export const FORUM_LIGHTING_KEY = 'permagent.forum.lighting';
function valid(value: unknown): value is ForumLightingPreference {
  return value === 'system' || value === 'day' || value === 'night';
}

/** This view's lighting override never changes the application's theme. */
export function useForumLighting() {
  const [lighting, setLighting] = useState<ForumLightingPreference>(() => {
    try { const saved = localStorage.getItem(FORUM_LIGHTING_KEY); return valid(saved) ? saved : 'system'; }
    catch { return 'system'; }
  });
  const chooseLighting = useCallback((next: ForumLightingPreference) => {
    if (!valid(next)) return;
    setLighting(next);
    try { localStorage.setItem(FORUM_LIGHTING_KEY, next); } catch { /* usable for this visit */ }
  }, []);
  return [lighting, chooseLighting] as const;
}
