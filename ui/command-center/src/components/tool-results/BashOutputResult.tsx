interface BashOutputResultProps {
  stdout?: string;
  stderr?: string;
  exitCode?: number;
}

export function BashOutputResult({ stdout, stderr, exitCode }: BashOutputResultProps) {
  return (
    <div className="space-y-1.5">
      {stdout && (
        <div>
          <div className="text-[9px] font-mono uppercase text-dark-muted mb-0.5">stdout</div>
          <pre className="rounded bg-black/40 p-2 font-mono text-[11px] text-emerald-300/90 overflow-x-auto max-h-[200px] overflow-y-auto whitespace-pre-wrap">
            {stdout}
          </pre>
        </div>
      )}
      {stderr && (
        <div>
          <div className="text-[9px] font-mono uppercase text-dark-muted mb-0.5">stderr</div>
          <pre className="rounded bg-black/40 p-2 font-mono text-[11px] text-red-300/90 overflow-x-auto max-h-[200px] overflow-y-auto whitespace-pre-wrap">
            {stderr}
          </pre>
        </div>
      )}
      {exitCode !== undefined && (
        <div className="text-[10px] font-mono text-dark-muted">
          exit code: <span className={exitCode === 0 ? 'text-emerald-400' : 'text-red-400'}>{exitCode}</span>
        </div>
      )}
    </div>
  );
}
