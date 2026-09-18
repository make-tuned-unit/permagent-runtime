import { memo, useCallback, type CSSProperties } from 'react';
import { FiAlertCircle, FiCheck, FiCopy } from 'react-icons/fi';
import { useCommandCenter, type ChatMessage } from '../../lib/store';
import { MessageRenderer } from './MessageRenderer';
import { CitationMarker } from '../awareness/CitationMarker';
import { useCopyToClipboard } from '../../lib/clipboard';
import { dispatchBody } from './dispatchBody';
import { font, space } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

import { Tooltip } from '../common/Tooltip';
function MessageBubbleInner({ message }: { message: ChatMessage }) {
  const { colors } = useTheme();
  const agentName = useCommandCenter(s => s.agentName);
  const isUser = message.role === 'user';
  const isSystem = message.role === 'system';
  const ctx = message.context_attached;

  const { state: copyState, copy } = useCopyToClipboard();
  const handleCopy = useCallback(() => {
    void copy(dispatchBody(message.content));
  }, [copy, message.content]);
  // The reader wrote their own messages; the thing worth lifting out of the
  // transcript is what the agent drafted for them.
  const canCopy = !isUser && !!message.content?.trim();

  const bubbleStyle = isUser
    ? { backgroundColor: colors.userBubble, border: `1px solid ${colors.purple}30` }
    : { backgroundColor: colors.surface, boxShadow: colors.cardShadow, ...(colors.cardHighlight ? { boxShadow: `${colors.cardShadow}, ${colors.cardHighlight}` } : {}) };

  return (
    <div className={`flex ${isUser ? 'justify-end' : 'justify-start'}`}>
      <div className="max-w-[85%] rounded-xl px-3.5 py-2.5" style={{ ...bubbleStyle, overflowWrap: 'break-word', wordBreak: 'break-word', minWidth: 0 }}>
        <div className="flex items-center gap-2 mb-1">
          <span
            className="text-[11px]"
            style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
          >
            {isUser ? 'You' : isSystem ? 'System' : agentName}
          </span>
          <span
            className="text-[10px]"
            style={{ fontFamily: font.mono, color: colors.textDim }}
          >
            {new Date(message.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
          </span>

          {canCopy && (
            <Tooltip content="Copy message">
              <Button
                colors={colors}
                variant="bare"
                type="button"
                onClick={handleCopy}
                aria-label="Copy message"
                // Dim rather than hidden. A hover-only control is the house
                // pattern for code blocks, but the report here was "I have to
                // select it and then copy" — an affordance nobody found. It has
                // to be visible without knowing to hover for it. The dim-to-full
                // is expressed in CSS rather than by JS mouse handlers writing to
                // `currentTarget.style`, which is what the primitive replaces.
                className={`ml-auto shrink-0 ${copyState === 'idle' ? 'opacity-[0.55] hover:opacity-100' : 'opacity-100'}`}
                style={{
                  '--pa-btn-fg': copyState === 'failed' ? colors.danger : copyState === 'copied' ? colors.success : colors.textDim,
                  '--pa-btn-bg-hover': 'transparent',
                  '--pa-btn-pad': '0',
                  fontFamily: font.mono,
                  fontSize: 10,
                  gap: space.xs,
                } as CSSProperties}
              >
                {copyState === 'copied' ? <><FiCheck size={11} /> Copied</>
                  : copyState === 'failed' ? <><FiAlertCircle size={11} /> Copy failed</>
                  : <FiCopy size={11} />}
              </Button>
            </Tooltip>
          )}

          {/* Announced to screen readers, which cannot see the icon swap above.
              Outside the button so its accessible name stays stable. */}
          <span role="status" aria-live="polite" className="sr-only">
            {copyState === 'copied' ? 'Message copied to clipboard'
              : copyState === 'failed' ? 'Could not copy the message'
              : ''}
          </span>
        </div>

        {isSystem ? (
          <div
            className="text-[12px] leading-relaxed whitespace-pre-wrap"
            style={{ fontFamily: font.body, color: colors.textMuted }}
          >
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

/** Every streamed delta replaces ONE message object; the store's `.map` hands
 *  back the same object for every other message. Without this memo React still
 *  re-rendered all of them, and each assistant bubble re-ran react-markdown over
 *  its whole body — measured at 5 full markdown parses per delta in an 8-message
 *  conversation, growing linearly with history length. Reference equality is the
 *  right comparator precisely because the store never mutates a message in
 *  place; a custom comparator would only mask a store that started to. */
export const MessageBubble = memo(MessageBubbleInner);
