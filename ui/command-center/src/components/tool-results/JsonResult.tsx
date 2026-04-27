import { useState, useCallback } from 'react';
import { JsonView, darkStyles } from 'react-json-view-lite';
import 'react-json-view-lite/dist/index.css';
import { FiCopy, FiCheck } from 'react-icons/fi';

interface JsonResultProps {
  data: unknown;
}

export function JsonResult({ data }: JsonResultProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(JSON.stringify(data, null, 2)).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }, [data]);

  return (
    <div className="relative group">
      <button
        onClick={handleCopy}
        className="absolute top-1 right-1 text-[10px] font-mono text-dark-muted hover:text-dark-text transition opacity-0 group-hover:opacity-100 flex items-center gap-1 z-10"
      >
        {copied ? <><FiCheck size={10} className="text-emerald-400" /> Copied</> : <><FiCopy size={10} /> Copy JSON</>}
      </button>
      <div className="overflow-x-auto max-h-[300px] overflow-y-auto text-[11px] [&_*]:!font-mono">
        <JsonView data={data as object} style={darkStyles} />
      </div>
    </div>
  );
}
