import { useState, useEffect, useCallback } from 'react';
import { ArrowLeft } from 'lucide-react';
import { useCommandCenter } from '../../lib/store';
import { ProvidersSection } from './ProvidersSection';

type SettingsTab = 'providers' | 'integrations' | 'about';

export function SettingsView() {
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const [activeTab, setActiveTab] = useState<SettingsTab>('providers');

  const dismiss = useCallback(() => setActivePanel('chat'), [setActivePanel]);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        e.preventDefault();
        dismiss();
      }
    }
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [dismiss]);

  const tabs: { id: SettingsTab; label: string }[] = [
    { id: 'providers', label: 'Providers' },
    { id: 'integrations', label: 'Integrations' },
    { id: 'about', label: 'About' },
  ];

  return (
    <div className="flex flex-col h-full bg-[#0B1120] text-dark-text overflow-y-auto">
      <div className="border-b border-dark-border px-6 py-4 flex items-center gap-3">
        <button
          onClick={dismiss}
          className="p-1.5 rounded hover:bg-white/5 text-dark-muted hover:text-dark-text transition-colors"
          title="Back to workspace (Esc)"
        >
          <ArrowLeft size={16} />
        </button>
        <div>
          <h1 className="text-lg font-semibold">Settings</h1>
          <p className="text-xs text-dark-muted mt-0.5">Manage providers, integrations, and preferences</p>
        </div>
      </div>

      <div className="border-b border-dark-border px-6">
        <div className="flex gap-0">
          {tabs.map(tab => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`px-4 py-2.5 text-xs font-mono transition border-b-2 ${
                activeTab === tab.id
                  ? 'text-accent border-accent'
                  : 'text-dark-muted border-transparent hover:text-dark-text'
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>

      <div className="p-6 max-w-2xl">
        {activeTab === 'providers' && <ProvidersSection />}
        {activeTab === 'integrations' && <IntegrationsPlaceholder />}
        {activeTab === 'about' && <AboutSection />}
      </div>
    </div>
  );
}

function IntegrationsPlaceholder() {
  return (
    <div className="text-xs text-dark-muted font-mono py-8 text-center">
      Integration management coming in Phase 2 Track 6.
    </div>
  );
}

function AboutSection() {
  return (
    <div className="rounded-lg border border-dark-border bg-[#111827] p-5 space-y-3">
      <h2 className="font-semibold">Permagent</h2>
      <div className="text-xs text-dark-muted space-y-1">
        <p>Persistent AI agent runtime with spectral memory.</p>
        <p>Version: 0.1.0 (Phase 2)</p>
      </div>
    </div>
  );
}
