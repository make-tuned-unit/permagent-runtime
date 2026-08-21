/**
 * Circular face for a person: photo when we have a safe http(s) URL,
 * initials otherwise. Clicking the face opens the detail panel on the graph.
 */

import { useEffect, useState, type CSSProperties } from 'react';
import { font } from '../../styles/tokens';
import { personInitials, safePhotoUrl } from './peopleFace';

export function PersonFace({
  name,
  photoUrl,
  size,
  accent,
  onClick,
  dimmed,
}: {
  name: string;
  photoUrl: string | null;
  size: number;
  accent: string;
  onClick?: () => void;
  dimmed?: boolean;
}) {
  const src = safePhotoUrl(photoUrl);
  const [broken, setBroken] = useState(false);
  useEffect(() => { setBroken(false); }, [src]);
  const showPhoto = Boolean(src) && !broken;
  const style: CSSProperties = {
    width: size,
    height: size,
    borderRadius: '50%',
    padding: 0,
    border: `2px solid ${accent}`,
    overflow: 'hidden',
    cursor: onClick ? 'pointer' : 'default',
    background: 'rgba(8,10,16,0.88)',
    display: 'grid',
    placeItems: 'center',
    fontFamily: font.body,
    fontSize: Math.max(11, Math.round(size * 0.36)),
    fontWeight: 600,
    color: accent,
    boxShadow: '0 6px 16px rgba(0,0,0,0.35)',
    flexShrink: 0,
    opacity: dimmed ? 0.42 : 1,
  };
  const inner = showPhoto ? (
    <img
      src={src!}
      alt=""
      referrerPolicy="no-referrer"
      onError={() => setBroken(true)}
      style={{ width: '100%', height: '100%', objectFit: 'cover', display: 'block' }}
    />
  ) : (
    personInitials(name)
  );
  if (onClick) {
    return (
      <button
        type="button"
        onClick={e => { e.stopPropagation(); onClick(); }}
        aria-label={`Open ${name}`}
        title={name}
        style={style}
      >
        {inner}
      </button>
    );
  }
  return (
    <div aria-label={name} title={name} style={style}>
      {inner}
    </div>
  );
}
