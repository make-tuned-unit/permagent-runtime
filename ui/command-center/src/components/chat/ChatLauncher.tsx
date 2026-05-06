import { useEffect, useState } from 'react';
import { color, font, ease } from '../../styles/tokens';
import { api } from '../../lib/api';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

async function openChatWindow() {
  if (!isTauri) return;
  try {
    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');

    // If already open, focus it
    const existing = await WebviewWindow.getByLabel('chat');
    if (existing) {
      await existing.show();
      await existing.setFocus();
      return;
    }

    // Create new chat window
    const chatWindow = new WebviewWindow('chat', {
      url: 'index.html?view=chat',
      title: 'Permagent Chat',
      width: 480,
      height: 700,
      minWidth: 360,
      minHeight: 400,
      center: true,
      decorations: true,
      resizable: true,
      titleBarStyle: 'overlay',
      hiddenTitle: true,
    });

    chatWindow.once('tauri://error', (e) => {
      console.error('Chat window error:', e);
    });
  } catch (e) {
    console.error('Failed to open chat window:', e);
  }
}

export function ChatLauncher() {
  const [agentName, setAgentName] = useState('Agent');

  useEffect(() => {
    api.getIdentity().then(id => setAgentName(id.first_name)).catch(() => {});
  }, []);

  return (
    <button onClick={openChatWindow} style={{
      position: 'fixed', bottom: 20, right: 20, zIndex: 9999,
      display: 'flex', alignItems: 'center', gap: 10,
      padding: '12px 20px', borderRadius: 999,
      background: 'rgba(20,28,48,0.85)', backdropFilter: 'blur(16px)',
      border: `1px solid ${color.borderHi}`,
      color: color.cyan, cursor: 'pointer',
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
