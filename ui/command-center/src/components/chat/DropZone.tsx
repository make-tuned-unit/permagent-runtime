import { useState, useRef, useCallback, useEffect } from 'react';
import { FiUpload } from 'react-icons/fi';
import { registerDropHandler } from '../../lib/native-drag-drop';

interface DropZoneProps {
  onDrop: (files: File[]) => void;
  children: React.ReactNode;
}

const isTauri = '__TAURI_INTERNALS__' in window;

export function DropZone({ onDrop, children }: DropZoneProps) {
  const [dragging, setDragging] = useState(false);
  const counter = useRef(0);

  // Tauri native drag-drop bridge
  useEffect(() => {
    if (!isTauri) return;

    let cancelled = false;
    let cleanup: (() => void) | null = null;

    registerDropHandler({
      onEnter: () => { if (!cancelled) setDragging(true); },
      onLeave: () => { if (!cancelled) setDragging(false); },
      onDrop: (files) => { if (!cancelled) onDrop(files); },
    }).then(fn => {
      if (cancelled) fn();
      else cleanup = fn;
    });

    return () => {
      cancelled = true;
      cleanup?.();
    };
  }, [onDrop]);

  // HTML5 drag-drop handlers (browser fallback)
  const handleDragEnter = useCallback((e: React.DragEvent) => {
    if (isTauri) return;
    e.preventDefault();
    counter.current++;
    if (e.dataTransfer.types.includes('Files')) {
      setDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    if (isTauri) return;
    e.preventDefault();
    counter.current--;
    if (counter.current === 0) setDragging(false);
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    if (isTauri) return;
    e.preventDefault();
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
    if (isTauri) return;
    e.preventDefault();
    counter.current = 0;
    setDragging(false);
    const files = Array.from(e.dataTransfer.files);
    if (files.length > 0) onDrop(files);
  }, [onDrop]);

  return (
    <div
      className="relative h-full"
      onDragEnter={handleDragEnter}
      onDragLeave={handleDragLeave}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
    >
      {children}
      {dragging && (
        <div className="absolute inset-0 z-40 flex flex-col items-center justify-center bg-[#0A0E17]/90 border-2 border-dashed border-accent/50 rounded-xl m-2 pointer-events-none">
          <FiUpload size={32} className="text-accent/60 mb-2" />
          <span className="text-accent/80 font-mono text-sm">Drop files here</span>
        </div>
      )}
    </div>
  );
}
