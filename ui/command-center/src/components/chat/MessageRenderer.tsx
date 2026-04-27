import type { ChatMessage } from '../../lib/store';
import { MarkdownContent } from './MarkdownContent';
import { ImageMessage } from './ImageMessage';
import { AudioMessage } from './AudioMessage';
import { ToolResult } from '../tool-results/ToolResult';

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
  const isUser = message.role === 'user';

  // Separate attachments by type
  const imageAttachments = attachments?.filter(a => a.mime_type.startsWith('image/')) || [];
  const audioAttachments = attachments?.filter(a => a.mime_type.startsWith('audio/')) || [];

  return (
    <>
      {/* Text content */}
      {message.content && (
        isUser ? (
          <div className="font-mono text-[13px] leading-relaxed text-blue-200 whitespace-pre-wrap">
            {message.content}
          </div>
        ) : (
          <div className="font-mono text-[13px] leading-relaxed text-dark-text">
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
