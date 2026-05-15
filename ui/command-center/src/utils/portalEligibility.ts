import type { IdentityState } from '../services/identity';

export interface PortalEligibility {
  sealed: boolean;
  soulValid: boolean;
  freshlyVerified: boolean;
  hasBindings: boolean;
  ready: boolean;
  verifiedDaysAgo: number | null;
}

export function computePortalEligibility(state: IdentityState): PortalEligibility {
  const sealed = state.status === 'sealed';
  const soulValid = state.soulValid === true;

  let verifiedDaysAgo: number | null = null;
  let freshlyVerified = false;
  if (state.lastVerifiedAt) {
    verifiedDaysAgo = Math.floor(
      (Date.now() - new Date(state.lastVerifiedAt).getTime()) / (24 * 60 * 60 * 1000),
    );
    freshlyVerified = verifiedDaysAgo < 30;
  }

  const hasBindings = state.bindingsCount >= 1;
  const ready = sealed && soulValid && freshlyVerified && hasBindings;

  return { sealed, soulValid, freshlyVerified, hasBindings, ready, verifiedDaysAgo };
}
