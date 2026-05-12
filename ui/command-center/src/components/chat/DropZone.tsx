import { useState, useRef, useCallback, useEffect } from 'react';
import { FiUpload } from 'react-icons/fi';
import { setDropHandlers } from '../../lib/native-drag-drop';

interface DropZoneProps {
  onDrop: (files: File[]) => void;
  children: React.ReactNode;
}

const isTauri = '__TAURI_INTERNALS__' in window;

export function DropZone({ onDrop, children }: DropZoneProps) {
  const [dragging, setDragging] = useState(false);
  const counter = useRef(0);

  useEffect(() => {
    console.log(`[dropzone] mounted, isTauri=${isTauri}`);
  }, []);

  // Tauri native drag-drop bridge — synchronous handler swap, no race conditions
  useEffect(() => {
    if (!isTauri) return;

    console.log('[dropzone] registering native drop handlers');
    setDropHandlers({
      onEnter: () => {
        console.log('[dropzone] native onEnter → show overlay');
        setDragging(true);
      },
      onLeave: () => {
        console.log('[dropzone] native onLeave → hide overlay');
        setDragging(false);
      },
      onDrop: (files) => {
        console.log('[dropzone] native onDrop, file count:', files.length);
        onDrop(files);
      },
    });

    return () => {
      console.log('[dropzone] clearing native drop handlers');
      setDropHandlers(null);
    };
  }, [onDrop]);

  // HTML5 drag-drop handlers (browser fallback)
  const handleDragEnter = useCallback((e: React.DragEvent) => {
    if (isTauri) return;
    e.preventDefault();
    counter.current++;
    if (e.dataTransfer.types.includes('Files')) {
      console.log('[dropzone] HTML5 dragenter, files detected');
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
    console.log('[dropzone] HTML5 onDrop, file count:', files.length);
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
          <span className="text-accent/80 font-mono text-sm">Drop files to send to chat</span>
        </div>
      )}
    </div>
  );
}
