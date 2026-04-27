import { FiFile } from 'react-icons/fi';

interface FileReadResultProps {
  filename?: string;
  content?: string;
  truncated?: boolean;
}

export function FileReadResult({ filename, content, truncated }: FileReadResultProps) {
  const preview = content ? content.slice(0, 500) : '';

  return (
    <div className="space-y-1.5">
      {filename && (
        <div className="flex items-center gap-1.5 text-[11px] font-mono text-dark-text">
          <FiFile size={12} className="text-dark-muted" />
          {filename}
        </div>
      )}
      {preview && (
        <pre className="rounded bg-black/30 p-2 font-mono text-[10px] text-slate-300 overflow-x-auto max-h-[200px] overflow-y-auto whitespace-pre-wrap">
          {preview}
          {(truncated || (content && content.length > 500)) && (
            <span className="text-dark-muted">... (truncated)</span>
          )}
        </pre>
      )}
    </div>
  );
}
