import { useState, useRef, useCallback, useEffect } from 'react';
import { FiUpload } from 'react-icons/fi';
import { useTheme } from '../../styles/useTheme';
import { setDropHandlers } from '../../lib/native-drag-drop';

interface DropZoneProps {
  onDrop: (files: File[]) => void;
  children: React.ReactNode;
  disabled?: boolean;
}

/**
 * File drop zone that wraps workspace content.
 *
 * In Tauri v2, native drag-drop intercepts Finder file drops before HTML5 events
 * fire, so we register a Tauri onDragDropEvent handler (via native-drag-drop.ts)
 * that reads file paths through a Tauri command and converts them to File objects.
 * HTML5 drag handlers remain as a fallback for browser environments.
 *
 * Internal drags (e.g. Kanban card DnD) are ignored — their events pass through
 * to inner handlers without interference.
 */
export function DropZone({ onDrop, children, disabled = false }: DropZoneProps) {
  const { colors } = useTheme();
  const [dragging, setDragging] = useState(false);
  const counter = useRef(0);
  const onDropRef = useRef(onDrop);
  onDropRef.current = onDrop;

  // Register Tauri native drag-drop handler (for Finder file drops).
  // In non-Tauri environments this is a no-op.
  useEffect(() => {
    if (disabled) return;
    setDropHandlers({
      onEnter: () => setDragging(true),
      onLeave: () => setDragging(false),
      onDrop: (files) => {
        setDragging(false);
        onDropRef.current(files);
      },
    });
    return () => setDropHandlers(null);
  }, [disabled]);

  const isFileDrag = (e: React.DragEvent) => e.dataTransfer.types.includes('Files');

  // HTML5 handlers: fallback for browser environments (Tauri intercepts these)
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
    if (isFileDrag(e)) {
      e.preventDefault();
    }
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
    const files = Array.from(e.dataTransfer.files);
    if (files.length > 0) {
      e.preventDefault();
      counter.current = 0;
      setDragging(false);
      onDrop(files);
    } else {
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
