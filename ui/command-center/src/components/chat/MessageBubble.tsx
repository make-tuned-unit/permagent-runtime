import type { ChatMessage } from '../../lib/store';
import { MessageRenderer } from './MessageRenderer';

export function MessageBubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === 'user';
  const isSystem = message.role === 'system';

  return (
    <div className={`flex ${isUser ? 'justify-end' : 'justify-start'}`}>
      <div className={`max-w-[85%] rounded-xl px-3.5 py-2.5 ${
        isUser
          ? 'bg-[rgba(141,68,174,0.16)] border border-[rgba(141,68,174,0.30)]'
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

        {isSystem ? (
          <div className="font-mono text-[11px] leading-relaxed text-slate-500 whitespace-pre-wrap">
            {message.content}
          </div>
        ) : (
          <MessageRenderer message={message} />
        )}
      </div>
    </div>
  );
}
