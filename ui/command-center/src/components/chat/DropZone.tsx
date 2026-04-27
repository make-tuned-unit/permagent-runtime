import { useState, useRef, useCallback } from 'react';
import { FiUpload } from 'react-icons/fi';

interface DropZoneProps {
  onDrop: (files: File[]) => void;
  children: React.ReactNode;
}

export function DropZone({ onDrop, children }: DropZoneProps) {
  const [dragging, setDragging] = useState(false);
  const counter = useRef(0);

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    counter.current++;
    if (e.dataTransfer.types.includes('Files')) {
      setDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    counter.current--;
    if (counter.current === 0) setDragging(false);
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
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
