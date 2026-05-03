import { useEffect, useRef, useState, useCallback } from 'react';
import { color, font, ease, radius } from '../../styles/tokens';
import { Mobius, type MobiusState } from '../mobius/Mobius';
import { useDashboard, type InFlightSession, type RecentSession } from './useDashboard';
import { Stat, SectionTitle, StatusIcon } from './atoms';
import { useCommandCenter } from '../../lib/store';
import { MessageList } from '../chat/MessageList';
import { ChatInput } from '../chat/ChatInput';
import type { ChatInputHandle } from '../chat/ChatInput';
import { DropZone } from '../chat/DropZone';

function timeAgo(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const min = Math.floor(ms / 60000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min}m ago`;
  const hrs = Math.floor(min / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

export function Dashboard() {
  const { data, loading } = useDashboard();

  if (loading || !data) {
    return (
      <div style={{ width: '100%', height: '100%', background: color.bg, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <Mobius size={120} state="thinking" />
      </div>
    );
  }

  const { agent, stats, in_flight, recent } = data;
  const mobiusState = (agent.state === 'thinking' ? 'thinking' : 'idle') as MobiusState;

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative' }}>
      <div style={{
        width: '100%', height: '100%', overflowY: 'auto',
        background: 'radial-gradient(120% 80% at 50% 0%, #142035 0%, #0B1220 50%, #050810 100%)',
        padding: '28px 32px 40px',
      }}>
        {/* Hero + Stats row */}
        <div style={{ display: 'grid', gridTemplateColumns: '1.2fr 1fr', gap: 24, marginBottom: 24 }}>
          {/* Hero card */}
          <div style={{
            position: 'relative', overflow: 'hidden',
            padding: 24, borderRadius: radius.lg,
            background: 'linear-gradient(180deg, rgba(20,28,48,0.7), rgba(11,18,32,0.7))',
            border: `1px solid ${color.border}`, minHeight: 220,
            display: 'flex', alignItems: 'center', gap: 24,
          }}>
            <div style={{ flex: 1 }}>
              <div style={{
                fontFamily: font.body, fontSize: 11, fontWeight: 600,
                letterSpacing: '0.14em', textTransform: 'uppercase',
                color: color.cyan, marginBottom: 12,
              }}>
                Status — {agent.state}
              </div>
              <div style={{
                fontFamily: font.display, fontSize: 24, fontWeight: 600,
                letterSpacing: '-0.02em', lineHeight: 1.2, marginBottom: 10,
              }}>
                {agent.active_count > 0 ? (
                  <>{agent.name} is working on<br /><span style={{ color: color.cyan }}>{agent.active_count} {agent.active_count === 1 ? 'thing' : 'things'}</span> for you</>
                ) : (
                  <>{agent.name} is<br />ready</>
                )}
              </div>
              <div style={{ fontSize: 14, color: color.textMuted, lineHeight: 1.5, maxWidth: 360 }}>
                {agent.active_count > 0
                  ? `Working across ${agent.active_count} session${agent.active_count > 1 ? 's' : ''}`
                  : 'Ready when you are.'}
              </div>
            </div>
            <div style={{ flex: '0 0 auto' }}>
              <Mobius size={96} state={mobiusState} glow={1} />
            </div>
          </div>

          {/* Stats grid */}
          <div style={{
            padding: 24, borderRadius: radius.lg,
            background: 'rgba(20,28,48,0.5)',
            border: `1px solid ${color.border}`,
            display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20,
          }}>
            <Stat label="Sessions today" value={stats.sessions_today} />
            <Stat label="Total sessions" value={stats.sessions_total} />
            <Stat label="Memory nodes" value={stats.memory_count} />
            <Stat label="New today" value={stats.memory_delta_today} delta={stats.memory_delta_today > 0 ? `+${stats.memory_delta_today}` : undefined} cyan />
          </div>
        </div>

        {/* In flight */}
        {in_flight.length > 0 && (
          <div style={{ marginBottom: 24 }}>
            <SectionTitle title="In flight" right={`${in_flight.length} active`} />
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 14 }}>
              {in_flight.map(task => <TaskCard key={task.id} task={task} />)}
            </div>
          </div>
        )}

        {/* Recent */}
        {recent.length > 0 && (
          <div>
            <SectionTitle title="Recent" right="last 24h" />
            <div style={{
              borderRadius: radius.lg, background: 'rgba(20,28,48,0.4)',
              border: `1px solid ${color.border}`, overflow: 'hidden',
            }}>
              {recent.map((item, i) => (
                <ActivityItem key={item.id} item={item} isLast={i === recent.length - 1} />
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Floating chat widget */}
      <HomeChat agentName={agent.name} />
    </div>
  );
}

function TaskCard({ task }: { task: InFlightSession }) {
  const mobiusState = (task.state === 'speaking' ? 'speaking' : 'thinking') as MobiusState;
  return (
    <div style={{
      padding: 18, borderRadius: radius.md,
      background: 'rgba(20,28,48,0.55)',
      border: `1px solid ${color.border}`,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 12 }}>
        <Mobius size={36} state={mobiusState} logoMode />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{
            fontFamily: font.body, fontSize: 13, fontWeight: 600, color: color.text,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>{task.title}</div>
          <div style={{ fontFamily: font.mono, fontSize: 11, color: color.textDim }}>
            Started {timeAgo(task.started_at)}
          </div>
        </div>
      </div>
      <div style={{
        height: 4, borderRadius: 999, background: 'rgba(255,255,255,0.06)',
        overflow: 'hidden',
      }}>
        <div style={{
          height: '100%', borderRadius: 999,
          width: `${Math.max(2, task.progress * 100)}%`,
          background: 'linear-gradient(90deg, #00D5FF, #A855F7)',
          boxShadow: '0 0 8px rgba(0,213,255,0.5)',
          transition: `width 300ms ${ease.out}`,
        }} />
      </div>
    </div>
  );
}

function ActivityItem({ item, isLast }: { item: RecentSession; isLast: boolean }) {
  const statusColor: Record<string, string> = {
    completed: '#5BD17F',
    paused: color.danger,
    awaiting_input: color.cyan,
  };
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: 16, padding: '14px 18px',
      borderBottom: isLast ? 'none' : `1px solid ${color.border}`,
    }}>
      <StatusIcon state={item.state} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{
          fontFamily: font.body, fontSize: 14, fontWeight: 500, color: color.text,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{item.title}</div>
        <div style={{ fontSize: 12, color: color.textMuted, marginTop: 2 }}>
          {timeAgo(item.ended_at)}
        </div>
      </div>
      <span style={{
        fontFamily: font.body, fontSize: 11, fontWeight: 600,
        letterSpacing: '0.06em', textTransform: 'uppercase',
        color: statusColor[item.state] || color.textMuted,
      }}>{item.state.replace('_', ' ')}</span>
    </div>
  );
}

function HomeChat({ agentName }: { agentName: string }) {
  const [open, setOpen] = useState(false);
  const ensureSession = useCommandCenter(s => s.ensureSession);
  const connectSession = useCommandCenter(s => s.connectSession);
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);
  const chatInputRef = useRef<ChatInputHandle>(null);
  const connectedRef = useRef(false);

  const handleDrop = useCallback((files: File[]) => {
    chatInputRef.current?.addFiles(files);
  }, []);

  // Connect session lazily when widget opens
  useEffect(() => {
    if (!open || connectedRef.current) return;
    connectedRef.current = true;
    (async () => {
      const sessionId = await ensureSession();
      if (sessionId) {
        await loadSessionMessages(sessionId);
        connectSession(sessionId);
      }
    })();
    return () => {
      useCommandCenter.getState().disconnectSession();
      connectedRef.current = false;
    };
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  // Collapsed: floating button
  if (!open) {
    return (
      <button onClick={() => setOpen(true)} style={{
        position: 'absolute', bottom: 20, right: 20, zIndex: 20,
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

  // Expanded: floating chat panel
  return (
    <DropZone onDrop={handleDrop}>
      <div style={{
        position: 'absolute', bottom: 20, right: 20, zIndex: 20,
        width: 380, height: 520,
        borderRadius: radius.lg,
        background: 'rgba(11,18,32,0.95)', backdropFilter: 'blur(24px)',
        border: `1px solid ${color.borderHi}`,
        boxShadow: '0 16px 48px rgba(0,0,0,0.6), 0 0 0 1px rgba(0,213,255,0.08)',
        display: 'flex', flexDirection: 'column',
        overflow: 'hidden',
      }}>
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10,
          padding: '12px 16px',
          borderBottom: `1px solid ${color.border}`,
          flexShrink: 0,
        }}>
          <Mobius size={22} state="idle" glow={0.5} />
          <span style={{ fontFamily: font.display, fontSize: 13, fontWeight: 600, color: color.text, flex: 1 }}>
            Chat with {agentName}
          </span>
          <button onClick={() => setOpen(false)} style={{
            width: 24, height: 24, borderRadius: 6,
            background: 'transparent', border: 'none',
            color: color.textMuted, cursor: 'pointer',
            display: 'grid', placeItems: 'center', fontSize: 16,
          }}>×</button>
        </div>

        {/* Messages */}
        <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
          <MessageList />
        </div>

        {/* Input */}
        <ChatInput ref={chatInputRef} />
      </div>
    </DropZone>
  );
}
