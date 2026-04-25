import { useEffect } from 'react';
import { useCommandCenter } from './lib/store';
import { Sidebar } from './components/sidebar/Sidebar';
import { ChatView } from './components/chat/ChatView';
import { SkillsPanel } from './components/skills/SkillsPanel';
import { EventLogView } from './components/events/EventLogView';
import { SettingsView } from './components/settings/SettingsView';
import { TerminalManager } from './components/terminal/TerminalManager';

function MainContent() {
  const activePanel = useCommandCenter(s => s.activePanel);

  switch (activePanel) {
    case 'chat':
      return <ChatView />;
    case 'skills':
      return <SkillsPanel />;
    case 'terminal':
      return <TerminalManager />;
    case 'events':
      return <EventLogView />;
    case 'settings':
      return <SettingsView />;
  }
}

function App() {
  const connect = useCommandCenter(s => s.connect);
  const disconnect = useCommandCenter(s => s.disconnect);

  useEffect(() => {
    connect();
    return () => disconnect();
  }, [connect, disconnect]);

  return (
    <div className="flex h-screen bg-[#0B1120]">
      <Sidebar />
      <main className="flex-1 min-w-0 overflow-hidden">
        <MainContent />
      </main>
    </div>
  );
}

export default App;
