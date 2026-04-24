import type { ChatMessage } from '../../lib/store';
import { ToolCallCard } from './ToolCallCard';

function renderMarkdown(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/```([\s\S]*?)```/g, '<pre class="rounded bg-black/30 p-2 my-1 overflow-x-auto"><code>$1</code></pre>')
    .replace(/`([^`]+)`/g, '<code class="rounded bg-black/30 px-1 py-0.5 text-accent">$1</code>')
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/\*(.+?)\*/g, '<em>$1</em>')
    .replace(/\n/g, '<br/>');
}

export function MessageBubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === 'user';
  const isSystem = message.role === 'system';

  return (
    <div className={`flex ${isUser ? 'justify-end' : 'justify-start'}`}>
      <div className={`max-w-[85%] rounded-xl px-3.5 py-2.5 ${
        isUser
          ? 'bg-blue-600/15 border border-blue-500/20'
          : isSystem
          ? 'bg-slate-800/30'
          : 'bg-dark-surface'
      }`}>
        <div className="flex items-center gap-2 mb-1">
          <span className="text-[10px] font-mono text-dark-muted">
            {isUser ? 'You' : isSystem ? 'System' : 'Agent'}
          </span>
          <span className="text-[10px] font-mono text-dark-muted/50">
            {new Date(message.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
          </span>
        </div>

        <div
          className={`font-mono text-[13px] leading-relaxed ${
            isUser
              ? 'text-blue-200'
              : isSystem
              ? 'text-slate-500 text-[11px]'
              : 'text-dark-text'
          }`}
          dangerouslySetInnerHTML={{ __html: renderMarkdown(message.content) }}
        />

        {message.tool_calls && message.tool_calls.length > 0 && (
          <div className="mt-2 space-y-1">
            {message.tool_calls.map((tc, i) => (
              <ToolCallCard key={`${message.id}-tc-${i}`} call={tc} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
