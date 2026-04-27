interface AudioMessageProps {
  src: string;
  filename: string;
}

export function AudioMessage({ src, filename }: AudioMessageProps) {
  return (
    <div className="flex flex-col gap-1 my-1">
      <span className="text-[10px] font-mono text-dark-muted truncate max-w-[300px]">{filename}</span>
      <audio controls preload="metadata" className="max-w-[400px] h-8">
        <source src={src} />
      </audio>
    </div>
  );
}
