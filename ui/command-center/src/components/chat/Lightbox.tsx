import { useEffect, useState, useCallback, type CSSProperties } from 'react';
import { FiX, FiChevronLeft, FiChevronRight } from 'react-icons/fi';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

interface LightboxProps {
  images: string[];
  startIndex: number;
  onClose: () => void;
}

/** The lightbox floats over its own black scrim, so its controls are white
 *  regardless of theme — the palette below is deliberately not tokenised. */
const overlayBtn: CSSProperties = {
  '--pa-btn-fg': 'rgba(255,255,255,0.7)',
  '--pa-btn-fg-hover': '#FFFFFF',
  '--pa-btn-bg-hover': 'transparent',
  '--pa-btn-bg-active': 'transparent',
  '--pa-btn-pad': '0',
} as CSSProperties;

export function Lightbox({ images, startIndex, onClose }: LightboxProps) {
  const { colors } = useTheme();
  const [index, setIndex] = useState(startIndex);
  const multi = images.length > 1;

  const prev = useCallback(() => setIndex(i => (i > 0 ? i - 1 : images.length - 1)), [images.length]);
  const next = useCallback(() => setIndex(i => (i < images.length - 1 ? i + 1 : 0)), [images.length]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
      if (e.key === 'ArrowLeft') prev();
      if (e.key === 'ArrowRight') next();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose, prev, next]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-sm"
      onClick={onClose}
    >
      <Button
        colors={colors}
        variant="bare"
        onClick={onClose}
        aria-label="Close image viewer"
        className="absolute top-4 right-4 z-10"
        style={overlayBtn}
      >
        <FiX size={24} />
      </Button>

      {multi && (
        <>
          <Button
            colors={colors}
            variant="bare"
            onClick={(e) => { e.stopPropagation(); prev(); }}
            aria-label="Previous image"
            className="absolute left-4 z-10"
            style={overlayBtn}
          >
            <FiChevronLeft size={32} />
          </Button>
          <Button
            colors={colors}
            variant="bare"
            onClick={(e) => { e.stopPropagation(); next(); }}
            aria-label="Next image"
            className="absolute right-4 z-10"
            style={overlayBtn}
          >
            <FiChevronRight size={32} />
          </Button>
        </>
      )}

      <img
        src={images[index]}
        alt=""
        className="max-h-[90vh] max-w-[90vw] object-contain rounded-lg shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      />

      {multi && (
        <div className="absolute bottom-4 text-white/50 text-sm font-mono">
          {index + 1} / {images.length}
        </div>
      )}
    </div>
  );
}
