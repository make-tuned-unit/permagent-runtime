import { FiX, FiFile } from 'react-icons/fi';

interface AttachmentChipProps {
  filename: string;
  onRemove: () => void;
}

export function AttachmentChip({ filename, onRemove }: AttachmentChipProps) {
  return (
    <div className="flex items-center gap-1.5 rounded-md bg-[#161b22] border border-dark-border px-2 py-1 text-[11px] font-mono text-dark-text max-w-[200px]">
      <FiFile size={12} className="text-dark-muted shrink-0" />
      <span className="truncate">{filename}</span>
      <button
        onClick={onRemove}
        className="text-dark-muted hover:text-red-400 transition shrink-0 ml-0.5"
      >
        <FiX size={12} />
      </button>
    </div>
  );
}
