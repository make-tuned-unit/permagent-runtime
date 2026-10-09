import type { ProviderInfo } from '../../lib/store';
import type { ResolvedModelRoute } from '../../lib/api';

export type ChatSetupState = 'checking' | 'needs-model' | 'unavailable' | null;
export type ModelSetupReadiness = 'checking' | 'needs-model' | 'unavailable' | 'configured';

/** A registry read proves only configuration, never successful inference. */
export function modelSetupReadiness({
  providers, providerCheckKnown, providersError, chatRoute,
  sessionRouteEvidence, hasConversation,
}: {
  providers: ProviderInfo[];
  providerCheckKnown: boolean;
  providersError: boolean;
  chatRoute: ResolvedModelRoute | null;
  sessionRouteEvidence: boolean;
  hasConversation: boolean;
}): ModelSetupReadiness {
  if (!providerCheckKnown) return 'checking';
  if (providersError) return 'unavailable';
  // Older daemons lack this projection. A registry entry (notably `local`)
  // alone is not proof that Chat has a selected model.
  if (!chatRoute) return 'unavailable';
  const provider = chatRoute.provider.trim();
  const model = chatRoute.model.trim();
  if (chatRoute.source !== 'unconfigured' && provider && model) {
    return providers.some(entry => entry.name === provider && entry.isConfigured)
      ? 'configured' : 'unavailable';
  }
  // A saved conversation may restore its own provider even with no current
  // config route. Its route is uncertain, so leave sending available.
  if (sessionRouteEvidence || hasConversation) return 'unavailable';
  return 'needs-model';
}

/** Show setup only on an empty, settled chat whose model status is unresolved. */
export function chatSetupState({
  modelReadiness,
  historyLoaded, messageCount, streaming, sessionLoadError, daemonAuthError, voiceActive,
}: {
  modelReadiness: ModelSetupReadiness;
  historyLoaded: boolean;
  messageCount: number;
  streaming: boolean;
  sessionLoadError: string | null;
  daemonAuthError: string | null;
  voiceActive: boolean;
}): ChatSetupState {
  if (!historyLoaded || messageCount > 0 || streaming || sessionLoadError || daemonAuthError || voiceActive) return null;
  return modelReadiness === 'configured' ? null : modelReadiness;
}
