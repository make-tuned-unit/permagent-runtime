import { useCommandCenter, type ChatMessage } from '../../lib/store';
import { MessageRenderer } from './MessageRenderer';
import { CitationMarker } from '../awareness/CitationMarker';
import { useTheme } from '../../styles/useTheme';

export function MessageBubble({ message }: { message: ChatMessage }) {
  const { colors } = useTheme();
  const agentName = useCommandCenter(s => s.agentName);
  const isUser = message.role === 'user';
  const isSystem = message.role === 'system';
  const ctx = message.context_attached;

  const bubbleStyle = isUser
    ? { backgroundColor: colors.userBubble, border: `1px solid ${colors.purple}30` }
    : { backgroundColor: colors.surface };

  return (
    <div className={`flex ${isUser ? 'justify-end' : 'justify-start'}`}>
      <div className="max-w-[85%] rounded-xl px-3.5 py-2.5" style={{ ...bubbleStyle, overflowWrap: 'break-word', wordBreak: 'break-word', minWidth: 0 }}>
        <div className="flex items-center gap-2 mb-1">
          <span className="text-[10px] font-mono text-dark-muted">
            {isUser ? 'You' : isSystem ? 'System' : agentName}
          </span>
          <span className="text-[10px] font-mono text-dark-muted/50">
            {new Date(message.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
          </span>
        </div>

        {isSystem ? (
          <div className="font-mono text-[11px] leading-relaxed text-dark-muted whitespace-pre-wrap">
            {message.content}
          </div>
        ) : (
          <MessageRenderer message={message} />
        )}

        {ctx && (ctx.probed_memories.length > 0 || ctx.recalled_memories.length > 0) && (
          <div className="flex justify-end mt-1.5">
            <CitationMarker probed={ctx.probed_memories} recalled={ctx.recalled_memories} />
          </div>
        )}
      </div>
    </div>
  );
}
