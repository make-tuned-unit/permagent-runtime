export const SESSION_MODES = ["auto", "approve", "chat"] as const;
export type SessionMode = (typeof SESSION_MODES)[number];

const ALIASES: Record<string, SessionMode> = {
  auto: "auto",
  automatic: "auto",
  yolo: "auto",
  approve: "approve",
  ask: "approve",
  default: "approve",
  chat: "chat",
  plan: "chat",
};

export const MODE_LABEL: Record<SessionMode, string> = {
  auto: "auto",
  approve: "ask",
  chat: "chat",
};

export const MODE_SUMMARY: Record<SessionMode, string> = {
  auto: "run tools without asking",
  approve: "ask before every tool call",
  chat: "chat only — no tools",
};

export function parseSessionMode(input: string): SessionMode | null {
  const key = input.trim().toLowerCase().replace(/-/g, "_");
  return ALIASES[key] ?? null;
}

export function nextSessionMode(current: SessionMode): SessionMode {
  const i = SESSION_MODES.indexOf(current);
  return SESSION_MODES[(i + 1) % SESSION_MODES.length]!;
}

export function formatModeHelp(current: SessionMode): string {
  const rows = SESSION_MODES.map((m) => {
    const mark = m === current ? "*" : " ";
    return ` ${mark} ${MODE_LABEL[m].padEnd(6)} ${MODE_SUMMARY[m]}`;
  }).join("\n");
  return `Current: ${MODE_LABEL[current]}\n\n${rows}\n\n/mode auto|ask|chat\nshift+tab / ctrl+tab cycles (cmd+tab if the terminal delivers it)`;
}

/** Map our mode onto whatever ACP advertised, else the raw id. */
export function resolveAcpModeId(
  mode: SessionMode,
  available: Array<{ id: string }> | undefined,
): string {
  if (!available?.length) return mode;
  const wanted: Record<SessionMode, string[]> = {
    auto: ["auto", "yolo", "acceptedits", "auto_edit", "auto-accept"],
    approve: ["approve", "default", "normal", "ask"],
    chat: ["chat", "plan", "read-only", "readonly", "ask"],
  };
  const exact = available.find((m) => m.id.toLowerCase() === mode);
  if (exact) return exact.id;
  const aliases = wanted[mode];
  const fuzzy = available.find((m) =>
    aliases.some((a) => m.id.toLowerCase().replace(/[-_]/g, "") === a.replace(/[-_]/g, "")),
  );
  return fuzzy?.id ?? mode;
}

export function estimateTokensFromChars(chars: number): number {
  return Math.max(0, Math.round(chars / 4));
}

export function formatTokenCount(tokens: number): string {
  if (tokens < 1000) return `~${tokens}`;
  if (tokens < 10_000) return `~${(tokens / 1000).toFixed(1)}k`;
  return `~${Math.round(tokens / 1000)}k`;
}
