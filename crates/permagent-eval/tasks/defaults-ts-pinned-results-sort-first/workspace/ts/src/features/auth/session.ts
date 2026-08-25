import type { User } from "../../core/types.ts";

export interface Session {
  user: User;
  expiresAt: number;
}

export function isExpired(session: Session, now: number): boolean {
  return now >= session.expiresAt;
}
