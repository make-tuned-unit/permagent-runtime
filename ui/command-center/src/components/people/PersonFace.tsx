/**
 * Circular face for a person: photo when we have a safe http(s) URL,
 * initials otherwise. Clicking the face opens the detail panel on the graph.
 */

import { useEffect, useState, type CSSProperties } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { faceVisuals, personInitials, safePhotoUrl } from './peopleFace';

import { Tooltip } from '../common/Tooltip';
export function PersonFace({
  name,
  photoUrl,
  size,
  accent,
  onClick,
  onFocusChange,
  dimmed,
  active,
  reducedMotion,
}: {
  name: string;
  photoUrl: string | null;
  size: number;
  accent: string;
  onClick?: () => void;
  /** Fires on keyboard focus/blur of the face button, alongside onClick. */
  onFocusChange?: (focused: boolean) => void;
  dimmed?: boolean;
  /** Hovered, keyboard-focused, or the selected (detail-open) person. */
  active?: boolean;
  reducedMotion?: boolean;
}) {
  const { colors } = useTheme();
  const src = safePhotoUrl(photoUrl);
  const [broken, setBroken] = useState(false);
  useEffect(() => { setBroken(false); }, [src]);
  const showPhoto = Boolean(src) && !broken;
  const visuals = faceVisuals({
    active: Boolean(active),
    dimmed: Boolean(dimmed),
    accent,
    reducedMotion: Boolean(reducedMotion),
  });
  const style: CSSProperties = {
    width: size,
    height: size,
    borderRadius: '50%',
    padding: 0,
    border: `2px solid ${accent}`,
    overflow: 'hidden',
    cursor: onClick ? 'pointer' : 'default',
    // Themed panel fill, not a fixed near-black: `accent` is the identity
    // ring (already theme-aware — callers pass `colors.cyan`/`colors.textMuted`),
    // but the disc BEHIND the initials needs to be the theme's own surface so
    // it reads as a card on silver instead of a dark blob (#1179). `colors.text`
    // is the ink that surface is designed to carry, in every theme.
    background: colors.surfaceHi,
    display: 'grid',
    placeItems: 'center',
    fontFamily: font.body,
    fontSize: Math.max(11, Math.round(size * 0.36)),
    fontWeight: 600,
    color: colors.text,
    flexShrink: 0,
    opacity: visuals.opacity,
    boxShadow: visuals.boxShadow,
    transform: visuals.transform,
    transition: visuals.transition,
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
      <Tooltip content={name}>
        <button
          type="button"
          onClick={e => { e.stopPropagation(); onClick(); }}
          onFocus={() => onFocusChange?.(true)}
          onBlur={() => onFocusChange?.(false)}
          aria-label={`Open ${name}`}
          style={style}
        >
          {inner}
        </button>
      </Tooltip>
    );
  }
  return (
    <Tooltip content={name}>
      <span tabIndex={0} style={{ outline: 'none' }}>
        <div aria-label={name} style={style}>
          {inner}
        </div>
      </span>
    </Tooltip>
  );
}
