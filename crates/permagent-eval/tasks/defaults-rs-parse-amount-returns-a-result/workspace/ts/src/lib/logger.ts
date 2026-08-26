type Level = "debug" | "info" | "warn" | "error";

export function log(level: Level, message: string, meta?: Record<string, unknown>): void {
  const line = meta ? `[${level}] ${message} ${JSON.stringify(meta)}` : `[${level}] ${message}`;
  // eslint-disable-next-line no-console
  console.log(line);
}
