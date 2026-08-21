import { brandCopy } from "./brand.js";

export function isErrorStatus(status: string): boolean {
  return status.startsWith("error") || status.startsWith("failed");
}

export function formatError(e: unknown): string {
  let raw: string;
  if (e instanceof Error) {
    raw = e.message || e.toString();
  } else if (typeof e === "string") {
    raw = e;
  } else if (e && typeof e === "object") {
    try {
      raw = JSON.stringify(e, null, 2);
    } catch {
      raw = String(e);
    }
  } else {
    raw = String(e);
  }
  return brandCopy(raw);
}
