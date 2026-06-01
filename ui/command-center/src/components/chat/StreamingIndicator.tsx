import { useCommandCenter } from '../../lib/store';

export function StreamingIndicator() {
  const agentName = useCommandCenter(s => s.agentName);
  return (
    <div className="flex justify-start">
      <div className="rounded-xl bg-dark-surface px-3.5 py-2.5">
        <div className="flex items-center gap-1.5">
          <span className="text-[10px] font-mono text-dark-muted">{agentName}</span>
          <div className="flex gap-1 ml-1">
            <span className="h-1.5 w-1.5 rounded-full bg-accent animate-bounce" style={{ animationDelay: '0ms' }} />
            <span className="h-1.5 w-1.5 rounded-full bg-accent animate-bounce" style={{ animationDelay: '150ms' }} />
            <span className="h-1.5 w-1.5 rounded-full bg-accent animate-bounce" style={{ animationDelay: '300ms' }} />
          </div>
        </div>
      </div>
    </div>
  );
}
