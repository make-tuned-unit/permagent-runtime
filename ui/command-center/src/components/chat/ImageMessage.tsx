import { useState } from 'react';
import { Lightbox } from './Lightbox';

interface ImageMessageProps {
  src: string;
  alt?: string;
  allImages?: string[];
}

export function ImageMessage({ src, alt, allImages }: ImageMessageProps) {
  const [showLightbox, setShowLightbox] = useState(false);
  const startIndex = allImages ? allImages.indexOf(src) : 0;

  return (
    <>
      <img
        src={src}
        alt={alt || 'Attached image'}
        className="rounded-lg max-h-[400px] max-w-full cursor-pointer hover:opacity-90 transition my-1"
        onClick={() => setShowLightbox(true)}
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
