import { useState, useRef, useCallback } from 'react';
import { FiUpload } from 'react-icons/fi';
import { useTheme } from '../../styles/useTheme';

interface DropZoneProps {
  onDrop: (files: File[]) => void;
  children: React.ReactNode;
  disabled?: boolean;
}

/**
 * File drop zone that wraps workspace content. Uses HTML5 drag-and-drop events
 * exclusively (fileDropEnabled=false in Tauri config disables native interception).
 *
 * Only reacts to drags containing Files (external file drops from Finder).
 * Internal drags (e.g. Kanban card DnD) are ignored — their events pass through
 * to inner handlers without interference.
 */
export function DropZone({ onDrop, children, disabled = false }: DropZoneProps) {
  const { colors } = useTheme();
  const [dragging, setDragging] = useState(false);
  const counter = useRef(0);

  const isFileDrag = (e: React.DragEvent) => e.dataTransfer.types.includes('Files');

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    if (disabled) return;
    e.preventDefault();
    counter.current++;
    if (isFileDrag(e)) {
      setDragging(true);
    }
  }, [disabled]);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    counter.current--;
    if (counter.current === 0) setDragging(false);
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    // Only accept file drags at this level. For internal drags (card DnD),
    // let the inner element's preventDefault() handle drop-target validation.
    if (isFileDrag(e)) {
      e.preventDefault();
    }
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
    // Only process if there are actual files; internal card drops have no files.
    const files = Array.from(e.dataTransfer.files);
    if (files.length > 0) {
      e.preventDefault();
      counter.current = 0;
      setDragging(false);
      onDrop(files);
    } else {
      // Not a file drop — reset overlay state but let the event propagate
      // to inner handlers (already handled by bubbling order).
      counter.current = 0;
      setDragging(false);
    }
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
        <div className="absolute inset-0 z-40 flex flex-col items-center justify-center border-2 border-dashed border-accent/50 rounded-xl m-2 pointer-events-none" style={{ backgroundColor: colors.bg, opacity: 0.93 }}>
          <FiUpload size={32} className="text-accent/60 mb-2" />
          <span className="text-accent/80 font-mono text-sm">Drop files to send to chat</span>
        </div>
      )}
    </div>
  );
}
