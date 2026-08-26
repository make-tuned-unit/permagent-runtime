import type { User } from "../../core/types.ts";

export function canEditBilling(user: User): boolean {
  return user.isAdmin;
}

export function canDeleteAccount(user: User, targetId: string): boolean {
  return user.isAdmin || user.id === targetId;
}
