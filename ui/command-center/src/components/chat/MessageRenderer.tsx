import type { ChatMessage } from '../../lib/store';
import { MarkdownContent } from './MarkdownContent';
import { ImageMessage } from './ImageMessage';
import { AudioMessage } from './AudioMessage';
import { ToolResult } from '../tool-results/ToolResult';
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

  // Separate attachments by type
  const imageAttachments = attachments?.filter(a => a.mime_type.startsWith('image/')) || [];
  const audioAttachments = attachments?.filter(a => a.mime_type.startsWith('audio/')) || [];

  return (
    <>
      {/* Inline images (base64 from user's attached files) */}
      {message.images && message.images.length > 0 && (
        <div className="flex flex-wrap gap-2 mb-2">
          {message.images.map((img, i) => (
            <img
              key={`${message.id}-img-${i}`}
              src={`data:${img.mimeType};base64,${img.data}`}
              alt="Attached image"
              className="rounded-lg shadow-sm border border-slate-700/40 object-contain"
              style={{ maxWidth: 300, maxHeight: 300 }}
            />
          ))}
        </div>
      )}

      {/* Text content */}
      {message.content && (
        isUser ? (
          <div className="font-mono text-[13px] leading-relaxed whitespace-pre-wrap" style={{ color: colors.userBubbleText }}>
            {message.content}
          </div>
        ) : (
          <div className="font-mono text-[13px] leading-relaxed text-dark-text" style={{ overflowWrap: 'break-word', wordBreak: 'break-word' }}>
            <MarkdownContent content={message.content} />
          </div>
        )
      )}

      {/* Image attachments */}
      {imageAttachments.map(a => (
        <ImageMessage key={a.id} src={a.url} alt={a.filename} allImages={allImages} />
      ))}

      {/* Audio attachments */}
      {audioAttachments.map(a => (
        <AudioMessage key={a.id} src={a.url} filename={a.filename} />
      ))}

      {/* Tool calls */}
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
