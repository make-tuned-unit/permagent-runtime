import { Button } from '../common/Button';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';
import type { ChatSetupState } from './chatSetupState';

/** Deterministic route to the existing hosted and local model setup controls. */
export async function openChatModelSetup() {
  // The detached Chat webview has its own store and no Settings surface.
  // Reuse the main window's cross-window navigation seam, then surface it.
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window &&
      new URLSearchParams(window.location.search).get('view') === 'chat') {
    const { emit } = await import('@tauri-apps/api/event');
    await emit('app_navigate', { tool_type: 'settings', panel_type: 'overlay', section: 'models' });
    try {
      const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
      const main = await WebviewWindow.getByLabel('main');
      if (main) { await main.show(); await main.setFocus(); }
    } catch { /* navigation was delivered even if focus is unavailable */ }
    return;
  }
  const store = useCommandCenter.getState();
  store.setPendingSettingsSection('models');
  store.setActivePanel('settings');
}

export function ChatSetupCard({ state, onRetry }: { state: Exclude<ChatSetupState, null>; onRetry: () => void }) {
  const { colors } = useTheme();
  const checking = state === 'checking';
  const unavailable = state === 'unavailable';
  const title = checking ? 'Checking model setup…' : unavailable ? 'Model status is unavailable' : 'Choose a model for Chat';
  const description = checking
    ? 'Looking for a configured model before your first message.'
    : unavailable
      ? 'Chat could not confirm your model configuration. Retry the check, or review the existing setup in Settings.'
      : 'Chat needs a selected model before it can reply. Choose a hosted provider and model, or set up a model on this Mac.';

  return <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-4">
    <section
      aria-label="Chat model setup"
      data-testid="chat-setup-card"
      data-state={state}
      style={{
        width: '100%', maxWidth: 420, padding: space.huge,
        border: `1px solid ${colors.border}`, borderRadius: radius.lg,
        background: colors.surface, boxShadow: colors.cardShadow,
        fontFamily: font.body,
      }}
    >
      <p style={{ margin: 0, color: colors.textMuted, fontFamily: font.mono, fontSize: textSize.micro, letterSpacing: '0.08em' }}>CHAT SETUP</p>
      <h2 style={{ margin: `${space.md}px 0 ${space.sm}px`, color: colors.text, fontFamily: font.display, fontSize: textSize.title, fontWeight: 600, lineHeight: 1.25 }}>{title}</h2>
      <p role={checking ? 'status' : undefined} style={{ margin: 0, color: colors.textMuted, fontSize: textSize.small, lineHeight: 1.55 }}>{description}</p>
      <div className="flex flex-wrap items-center" style={{ gap: space.sm, marginTop: space.xxl }}>
        <Button type="button" colors={colors} variant="primary" flashSuccess={false} onClick={openChatModelSetup}>Open model setup</Button>
        {unavailable && <Button type="button" colors={colors} variant="ghost" flashSuccess={false} onClick={onRetry}>Retry check</Button>}
      </div>
      {!checking && <p style={{ margin: `${space.lg}px 0 0`, color: colors.textMuted, fontSize: textSize.caption, lineHeight: 1.5 }}>Settings → Models includes local models and a route to provider API keys. Opening setup does not send a message.</p>}
    </section>
  </div>;
}
