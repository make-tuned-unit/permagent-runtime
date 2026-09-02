/**
 * The still a social post carries.
 *
 * Split out of GrowView.tsx (R9), unchanged. It fetches through
 * `api.fetchGrowMediaBlob` rather than pointing an `src` at the media route,
 * because the daemon needs an auth header an `<img src>` cannot carry.
 */

import { useEffect, useState } from 'react';
import { radius } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { api } from '../../lib/api';

export function PostStill({
  projectId, cardId, filename, cacheKey, colors,
}: {
  projectId: string;
  cardId: string;
  filename: string;
  cacheKey?: string;
  colors: ThemeColors;
}) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    let objectUrl: string | null = null;
    api.fetchGrowMediaBlob(projectId, cardId, filename)
      .then((blob) => {
        if (!live) return;
        objectUrl = URL.createObjectURL(blob);
        setUrl(objectUrl);
      })
      .catch(() => { /* still generating or missing */ });
    return () => {
      live = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [projectId, cardId, filename, cacheKey]);
  if (!url) {
    return (
      <div style={{
        width: 72, height: 90, borderRadius: radius.sm,
        background: colors.bgDeeper, border: `1px solid ${colors.border}`,
      }} />
    );
  }
  return (
    <img
      src={url}
      alt=""
      style={{
        width: 72, height: 90, objectFit: 'cover', borderRadius: radius.sm,
        border: `1px solid ${colors.border}`,
      }}
    />
  );
}
