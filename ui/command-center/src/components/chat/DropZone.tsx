import { useState, useRef, useCallback, useEffect } from 'react';
import { FiUpload } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { setDropHandlers } from '../../lib/native-drag-drop';

interface DropZoneProps {
  onDrop: (files: File[]) => void;
  children: React.ReactNode;
  disabled?: boolean;
}

export function DropZone({ onDrop, children, disabled = false }: DropZoneProps) {
  const { colors } = useTheme();
  const [dragging, setDragging] = useState(false);
  const counter = useRef(0);
  const onDropRef = useRef(onDrop);
  onDropRef.current = onDrop;

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

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    if (disabled) return;
    e.preventDefault();
    counter.current++;
    if (isFileDrag(e)) setDragging(true);
  }, [disabled]);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    counter.current--;
    if (counter.current === 0) setDragging(false);
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    if (isFileDrag(e)) e.preventDefault();
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
        <div
          className="absolute inset-0 z-40 flex flex-col items-center justify-center rounded-xl m-2 pointer-events-none"
          style={{
            backgroundColor: colors.bg,
            opacity: 0.93,
            border: `2px dashed ${colors.cyan}80`,
          }}
        >
          <FiUpload size={32} className="mb-2" style={{ color: `${colors.cyan}99` }} />
          <span className="text-sm" style={{ fontFamily: font.mono, color: `${colors.cyan}CC` }}>Drop files to send to chat</span>
        </div>
      )}
    </div>
  );
}
