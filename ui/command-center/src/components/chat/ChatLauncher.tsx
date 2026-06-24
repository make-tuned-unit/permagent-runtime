import { useEffect, useState, useCallback } from 'react';
import { font, ease } from '../../styles/tokens';
import { api } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';
import { createChatWindow } from '../../lib/chatWindow';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export function ChatLauncher() {
  const { colors, theme } = useTheme();
  const [agentName, setAgentName] = useState('Agent');
  const [chatWindowOpen, setChatWindowOpen] = useState(false);

  useEffect(() => {
    api.getIdentity().then(id => setAgentName(id.first_name)).catch(() => {});
  }, []);

  // Check if chat window is already open on mount (handles main window reload)
  useEffect(() => {
    if (!isTauri) return;
    (async () => {
      try {
        const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const existing = await WebviewWindow.getByLabel('chat');
        if (existing) setChatWindowOpen(true);
      } catch { /* ignore */ }
    })();
  }, []);

  // Poll whether the chat window exists (handles user closing it via traffic light)
  useEffect(() => {
    if (!isTauri || !chatWindowOpen) return;
    const interval = setInterval(async () => {
      try {
        const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const existing = await WebviewWindow.getByLabel('chat');
        if (!existing) setChatWindowOpen(false);
      } catch { setChatWindowOpen(false); }
    }, 1000);
    return () => clearInterval(interval);
  }, [chatWindowOpen]);

  const openChatWindow = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');

      const existing = await WebviewWindow.getByLabel('chat');
      if (existing) {
        await existing.show();
        // The chat window is parented to the main window (see chatWindow.ts),
        // so it always sits above the main window's native browser child-webview
        // without floating above other apps — no always-on-top re-assert needed.
        await existing.setFocus();
        setChatWindowOpen(true);
        return;
      }

      const chatWindow = await createChatWindow(theme);

      chatWindow.once('tauri://created', async () => {
        setChatWindowOpen(true);
        // Ensure chat window comes to front above the main window
        await chatWindow.setFocus();
      });
      chatWindow.once('tauri://error', (e) => {
        console.error('Chat window error:', e);
        setChatWindowOpen(false);
      });
    } catch (e) {
      console.error('Failed to open chat window:', e);
    }
  }, [theme]);

  if (chatWindowOpen) return null;

  return (
    <button onClick={openChatWindow} style={{
      position: 'fixed', bottom: 20, right: 20, zIndex: 9999,
      display: 'flex', alignItems: 'center', gap: 10,
      padding: '12px 20px', borderRadius: 999,
      background: colors.surface, backdropFilter: 'blur(16px)',
      border: `1px solid ${colors.borderHi}`,
      color: colors.cyan, cursor: 'pointer',
      fontFamily: font.body, fontSize: 13, fontWeight: 600,
      boxShadow: '0 8px 32px rgba(0,0,0,0.5)',
      transition: `all 200ms ${ease.out}`,
    }}>
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">
        <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2v10z" />
      </svg>
      Chat with {agentName}
    </button>
  );
}
