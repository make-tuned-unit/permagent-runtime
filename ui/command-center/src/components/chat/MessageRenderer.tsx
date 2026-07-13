import type { ChatMessage } from '../../lib/store';
import { MarkdownContent } from './MarkdownContent';
import { ImageMessage } from './ImageMessage';
import { AudioMessage } from './AudioMessage';
import { ReasoningBlock } from './ReasoningBlock';
import { ToolResult } from '../tool-results/ToolResult';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface Attachment {
  id: string;
  filename: string;
  mime_type: string;
  url: string;
}

interface MessageRendererProps {
  message: ChatMessage;
  attachments?: Attachment[];
  allImages?: string[];
}

export function MessageRenderer({ message, attachments, allImages }: MessageRendererProps) {
  const { colors } = useTheme();
  const isUser = message.role === 'user';

  const imageAttachments = attachments?.filter(a => a.mime_type.startsWith('image/')) || [];
  const audioAttachments = attachments?.filter(a => a.mime_type.startsWith('audio/')) || [];

  return (
    <>
      {message.images && message.images.length > 0 && (
        <div className="flex flex-wrap gap-2 mb-2">
          {message.images.map((img, i) => (
            <img
              key={`${message.id}-img-${i}`}
              src={`data:${img.mimeType};base64,${img.data}`}
              alt="Attached image"
              className="rounded-lg shadow-sm object-contain"
              style={{ maxWidth: 300, maxHeight: 300, border: `1px solid ${colors.border}` }}
            />
          ))}
        </div>
      )}

      {!isUser && message.thinking && (
        <ReasoningBlock thinking={message.thinking} hasAnswer={!!message.content} />
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

      {imageAttachments.map(a => (
        <ImageMessage key={a.id} src={a.url} alt={a.filename} allImages={allImages} />
      ))}

      {audioAttachments.map(a => (
        <AudioMessage key={a.id} src={a.url} filename={a.filename} />
      ))}

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
