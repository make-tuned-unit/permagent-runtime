import { useState } from 'react';
import { FiImage } from 'react-icons/fi';
import { Lightbox } from './Lightbox';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface ImageMessageProps {
  src: string;
  alt?: string;
  allImages?: string[];
}

/** R2 (dagK UX standard, 2026-09-25): a broken `src` — corrupt base64, an
 *  empty attachment — used to draw the browser's own native broken-image
 *  glyph with no explanation, indistinguishable at a glance from a message
 *  that never had an image at all (an investigation into a harness bug on
 *  this component turned up an image that WAS valid but rendered nothing
 *  visible, which is what led here — this is the missing "error" state that
 *  case exposed the need for, not a fix for that case itself). */
export function ImageMessage({ src, alt, allImages }: ImageMessageProps) {
  const { colors } = useTheme();
  const [showLightbox, setShowLightbox] = useState(false);
  const [failed, setFailed] = useState(false);
  const startIndex = allImages ? allImages.indexOf(src) : 0;

  if (failed) {
    return (
      <div
        role="status"
        className="flex items-center gap-2 rounded-lg my-1"
        style={{
          padding: `${space.sm}px ${space.md}px`,
          backgroundColor: colors.surfaceHi,
          border: `1px solid ${colors.border}`,
          borderRadius: radius.md,
          color: colors.textMuted,
          fontFamily: font.body,
          fontSize: textSize.caption,
        }}
      >
        <FiImage size={14} style={{ flexShrink: 0 }} />
        Image failed to load
      </div>
    );
  }

  return (
    <>
      <img
        src={src}
        alt={alt || 'Attached image'}
        className="rounded-lg max-h-[400px] max-w-full cursor-pointer hover:opacity-90 transition my-1"
        onClick={() => setShowLightbox(true)}
        onError={() => setFailed(true)}
      />
      {showLightbox && (
        <Lightbox
          images={allImages || [src]}
          startIndex={Math.max(0, startIndex)}
          onClose={() => setShowLightbox(false)}
        />
      )}
    </>
  );
}
