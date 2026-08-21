export const DEFAULT_AUTO_TURNS = 12;
export const CONTINUE_PROMPT =
  "Continue. If the task is complete, stop and summarize what you did. If not, keep going.";

export interface AutonomousState {
  enabled: boolean;
  maxTurns: number;
  turnsUsed: number;
  gate: string | null;
  startedAt: number | null;
}

export function idleAutonomous(): AutonomousState {
  return {
    enabled: false,
    maxTurns: DEFAULT_AUTO_TURNS,
    turnsUsed: 0,
    gate: null,
    startedAt: null,
  };
}

export type AutonomousCommand =
  | { action: "status" }
  | { action: "on"; maxTurns?: number; gate?: string }
  | { action: "off" }
  | { action: "gate"; command: string }
  | { action: "error"; message: string };

export function parseAutonomousArgs(args: string): AutonomousCommand {
  const t = args.trim();
  if (!t || t.toLowerCase() === "status") return { action: "status" };
  const lower = t.toLowerCase();
  if (lower === "off" || lower === "stop") return { action: "off" };
  if (lower === "on") return { action: "on" };

  const onGate = /^on(?:\s+(\d+))?(?:\s+gate\s+(.+))$/i.exec(t);
  if (onGate) {
    return {
      action: "on",
      maxTurns: onGate[1] ? Number(onGate[1]) : undefined,
      gate: onGate[2]?.trim() || undefined,
    };
  }
  const onTurns = /^on\s+(\d+)$/i.exec(t);
  if (onTurns) return { action: "on", maxTurns: Number(onTurns[1]) };
  if (/^\d+$/.test(t)) return { action: "on", maxTurns: Number(t) };
  if (/^gate\s+\S/i.test(t)) {
    return { action: "gate", command: t.replace(/^gate\s+/i, "").trim() };
  }
  return {
    action: "error",
    message:
      "Usage: /autonomous on [turns] | off | status | gate <cmd>\nExample: /autonomous on 20 gate npm test",
  };
}

export function formatAutonomousStatus(state: AutonomousState): string {
  if (!state.enabled) {
    return `autonomous is off\nmax turns ${state.maxTurns}${
      state.gate ? `\ngate ${state.gate}` : ""
    }\n\n/autonomous on  to keep working after each turn`;
  }
  return [
    `autonomous is on  ${state.turnsUsed}/${state.maxTurns} continuations`,
    state.gate ? `gate  ${state.gate}` : "gate  (none)",
    "esc stops the turn and disables autonomous",
  ].join("\n");
}

export function shouldAutoContinue(
  state: AutonomousState,
  opts: { stopReason: string; queueEmpty: boolean; cancelled: boolean },
): { continue: false; reason?: string } | { continue: true } {
  if (!state.enabled) return { continue: false };
  if (opts.cancelled) return { continue: false, reason: "stopped" };
  if (!opts.queueEmpty) return { continue: false };
  if (opts.stopReason !== "end_turn") return { continue: false };
  if (state.turnsUsed >= state.maxTurns) {
    return { continue: false, reason: `paused — hit ${state.maxTurns} continuations` };
  }
  return { continue: true };
}

export function enableAutonomous(
  prev: AutonomousState,
  maxTurns?: number,
  gate?: string,
): AutonomousState {
  return {
    enabled: true,
    maxTurns: maxTurns && maxTurns > 0 ? maxTurns : prev.maxTurns,
    turnsUsed: 0,
    gate: gate !== undefined ? gate : prev.gate,
    startedAt: Date.now(),
  };
}

export async function runGateCommand(
  command: string,
  cwd: string,
): Promise<{ ok: boolean; output: string }> {
  const { execFile } = await import("node:child_process");
  const { promisify } = await import("node:util");
  const execFileAsync = promisify(execFile);
  try {
    const { stdout, stderr } = await execFileAsync("sh", ["-c", command], {
      cwd,
      timeout: 300_000,
      maxBuffer: 64 * 1024,
    });
    const output = [stdout, stderr].filter(Boolean).join("\n").trim();
    return { ok: true, output: output.slice(0, 4000) };
  } catch (e: unknown) {
    const err = e as { stdout?: string; stderr?: string; message?: string };
    const output = [err.stdout, err.stderr, err.message]
      .filter(Boolean)
      .join("\n")
      .trim();
    return { ok: false, output: (output || String(e)).slice(0, 4000) };
  }
}
