import type { ProviderEntry } from './providerChoices';
import { LOCAL_RUNTIME_PROVIDER } from './providerChoices';
import type { OllamaInventory } from './memoryStatus';

/**
 * One source of truth for "can this Mac run a local model", shared by the
 * model step and the review/extras steps. Four separate facts, never merged:
 * configured (the service offers and has saved the runtime), reachable (the
 * Ollama server answers), ready (reachable and at least one model installed),
 * inUse (it is the default provider).
 */
export interface LocalRuntimeFacts {
  offered: boolean;
  configured: boolean;
  /** null while unknown (still checking, or the check failed). */
  reachable: boolean | null;
  ready: boolean | null;
  inUse: boolean;
  /** The reachability read itself failed. */
  checkFailed: boolean;
}

export function localRuntimeFacts({ providers, ollama, checkFailed = false }: {
  providers: ProviderEntry[] | null;
  ollama: OllamaInventory | null;
  checkFailed?: boolean;
}): LocalRuntimeFacts {
  const entry = providers?.find(p => p.name === LOCAL_RUNTIME_PROVIDER);
  const reachable = checkFailed || !ollama ? null : ollama.reachable;
  return {
    offered: !!entry,
    configured: !!entry?.is_configured,
    reachable,
    ready: reachable === null ? null : reachable && ollama!.installed.length > 0,
    inUse: !!entry?.is_configured && !!entry?.is_default,
    checkFailed,
  };
}

/** The "On this Mac" card: its status line, whether it can be chosen, and why not. */
export function localRuntimeCard(facts: LocalRuntimeFacts, providersLoaded: boolean): { status: string; enabled: boolean } {
  if (!providersLoaded) return { status: 'Checking availability…', enabled: false };
  if (!facts.offered) return { status: 'Not offered by this service', enabled: false };
  if (facts.checkFailed) return { status: 'Ollama unavailable: could not check it', enabled: false };
  if (facts.reachable === null) return { status: 'Checking availability…', enabled: false };
  if (!facts.reachable) return { status: 'Ollama unavailable: not reachable on this Mac', enabled: false };
  if (!facts.ready) return { status: 'Ollama reachable · no models installed yet', enabled: true };
  return { status: facts.inUse ? 'Ollama ready · in use' : 'Ollama ready · models installed', enabled: true };
}
