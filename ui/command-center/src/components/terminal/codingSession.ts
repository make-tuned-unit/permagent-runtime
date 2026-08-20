// Coding-session capture — pure helpers, regression-tested.
//
// When a coding harness (Claude Code / Codex / the Permagent CLI) finishes in
// a terminal tab, the transcript tail is shipped to the daemon, summarized,
// and remembered in the Brain — so the agent knows what the user has been
// building without being told (reported gap, 2026-08-06).

/** The harness CLIs whose sessions are worth remembering. Matched on the
 *  command's first token (path prefixes allowed: `~/bin/claude` counts),
 *  so `git clone claude-thing` never false-positives. */
export function isHarnessCommand(command: string | null | undefined): boolean {
  if (!command) return false;
  const first = command.trim().split(/\s+/)[0] ?? '';
  const bin = first.split('/').pop() ?? '';
  return bin === 'claude' || bin === 'codex' || bin === 'permagent' || bin === 'cursor-agent';
}

/** Strip ANSI escapes + terminal control noise so the daemon summarizes
 *  prose, not cursor choreography. Covers CSI, OSC (BEL- and ST-terminated),
 *  charset selects, and bare C0 controls except newline/tab. */
export function stripAnsi(raw: string): string {
  return raw
    // OSC … BEL | OSC … ST
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, '')
    // CSI sequences
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, '')
    // Two-char escapes (charset selects, keypad modes, ESC 7/8 …)
    .replace(/\x1b[()#][A-Za-z0-9]/g, '')
    .replace(/\x1b[@-Z\\-_]/g, '')
    // Remaining C0 controls except \n and \t; CR dropped (TUI line paints)
    .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '')
    .replace(/\r/g, '');
}

/** Bound the transcript to its TAIL — the end holds the wrap-up. */
export function transcriptTail(text: string, maxChars = 24_000): string {
  return text.length <= maxChars ? text : text.slice(text.length - maxChars);
}

export interface CodingSessionPayload {
  transcript: string;
  cwd?: string;
  command?: string;
  duration_secs?: number;
}

/** Assemble the daemon payload; null when there is nothing worth sending. */
export function buildCodingSessionPayload(args: {
  rawTranscript: string;
  cwd?: string;
  command: string;
  spawnedAtMs: number;
  nowMs: number;
}): CodingSessionPayload | null {
  const transcript = transcriptTail(stripAnsi(args.rawTranscript)).trim();
  // A trivial transcript is a mis-fire (instant exit, --version, a typo) —
  // remembering it would be noise.
  if (transcript.length < 200) return null;
  return {
    transcript,
    cwd: args.cwd,
    command: args.command,
    duration_secs: Math.max(0, Math.round((args.nowMs - args.spawnedAtMs) / 1000)),
  };
}
