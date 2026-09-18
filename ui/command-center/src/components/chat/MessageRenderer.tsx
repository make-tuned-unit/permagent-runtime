import type { ChatMessage } from '../../lib/store';
import { MarkdownContent } from './MarkdownContent';
import { ImageMessage } from './ImageMessage';
import { ReasoningBlock } from './ReasoningBlock';
import { ToolResult } from '../tool-results/ToolResult';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface MessageRendererProps {
  message: ChatMessage;
}

export function MessageRenderer({ message }: MessageRendererProps) {
  const { colors } = useTheme();
  const isUser = message.role === 'user';

  // Inline images ride on the message as base64 (message.images). Render each
  // through ImageMessage so it is click-to-zoom via the shared Lightbox, with
  // gallery nav across every image in the message. 2026-07 wiring audit: this
  // was a bare <img> and ImageMessage/Lightbox were orphaned behind an
  // `attachments` prop the sole caller (MessageBubble) never passed.
  const inlineImages = (message.images ?? []).map(img => `data:${img.mimeType};base64,${img.data}`);

  return (
    <>
      {inlineImages.length > 0 && (
        <div className="flex flex-wrap gap-2 mb-2">
          {inlineImages.map((src, i) => (
            <ImageMessage
              key={`${message.id}-img-${i}`}
              src={src}
              alt="Attached image"
              allImages={inlineImages}
            />
          ))}
        </div>
      )}

      {!isUser && message.thinking && (
        <ReasoningBlock thinking={message.thinking} hasAnswer={!!message.content} />
      )}

      {/* The daemon speaking about itself, not the model answering. It is
          styled apart from the reply on purpose: "you are out of credit" is
          not something the agent said, and dressing it as prose would put
          words in the agent's mouth. */}
      {message.notice && (
        <div
          role="status"
          data-notice={message.notice.kind}
          className="text-[13px] leading-relaxed rounded-lg px-3 py-2 border"
          style={{
            fontFamily: font.body,
            color: colors.text,
            borderColor: message.notice.kind === 'creditsExhausted' ? colors.warning : colors.border,
            background: colors.surfaceHi,
            marginTop: message.content ? 8 : 0,
          }}
        >
          {message.notice.text}
        </div>
      )}

      {message.content && (
        isUser ? (
          <div
            className="text-[14px] leading-relaxed whitespace-pre-wrap"
            style={{ fontFamily: font.body, color: colors.userBubbleText }}
          >
            {message.content}
          </div>
        ) : (
          <div
            className="text-[14px] leading-relaxed"
            style={{ fontFamily: font.body, color: colors.text, overflowWrap: 'break-word', wordBreak: 'break-word' }}
          >
            <MarkdownContent content={message.content} />
          </div>
        )
      )}

      {message.tool_calls && message.tool_calls.length > 0 && (
        <div className="mt-2 space-y-1">
          {message.tool_calls.map((tc, i) => (
            <ToolResult key={`${message.id}-tc-${i}`} call={tc} />
          ))}
        </div>
      )}
    </>
  );
}
