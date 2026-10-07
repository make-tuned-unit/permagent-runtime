import type { ReactNode } from 'react';
import { FiHardDrive, FiKey, FiEyeOff, FiSend, FiSearch } from 'react-icons/fi';
import { font, radius, space, textSize, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { Panel } from './atoms';
import { StepFrame } from './StepFrame';

interface Props {
  onNext: () => void;
}

/**
 * What Permagent is, and — before anything is typed — what stays on this Mac
 * and what leaves it. Every claim here is checked against the code:
 * data under `~/.permagent` (config/paths.rs), keys in the keychain or a
 * password manager reference (Settings → Secrets), product analytics are an
 * explicit opt-in (routes/telemetry.rs), and a local model keeps prompts on
 * the machine.
 */
export function StepWelcome({ onNext }: Props) {
  return (
    <StepFrame
      lead={<div style={{ display: 'flex', justifyContent: 'center', marginBottom: space.xl }}><Mobius size={72} state="idle" /></div>}
      title="Welcome to Permagent"
      why="An agent that works on your Mac: it chats, runs tasks in terminals and the browser, and remembers what matters to you. Setup takes about two minutes."
      primary={{ label: 'Get started', autoFocus: true }}
      onPrimary={onNext}
      width={640}
    >
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))', gap: space.xl }}>
        <Panel>
          <PanelTitle>Stays on this Mac</PanelTitle>
          <Fact icon={<FiHardDrive />}>Your memory, chats and settings, in <Code>~/.permagent</Code>.</Fact>
          <Fact icon={<FiKey />}>API keys, in the macOS keychain or your password manager.</Fact>
          <Fact icon={<FiEyeOff />}>Usage analytics are off unless you turn them on in Settings.</Fact>
        </Panel>
        <Panel>
          <PanelTitle>Leaves this Mac</PanelTitle>
          <Fact icon={<FiSend />}>Your messages, to the model provider you choose and only to it. Choose a local model and they never leave.</Fact>
          <Fact icon={<FiSearch />}>Web searches, to the search service you connect, if you connect one.</Fact>
        </Panel>
      </div>
    </StepFrame>
  );
}

function PanelTitle({ children }: { children: ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{ ...type.label, fontFamily: font.body, color: colors.textMuted, marginBottom: space.lg }}>{children}</div>
  );
}

function Fact({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', gap: space.lg, alignItems: 'flex-start', marginTop: space.md }}>
      <span aria-hidden style={{ color: colors.cyan, flexShrink: 0, marginTop: space.xxs, display: 'inline-flex' }}>{icon}</span>
      <span style={{ fontFamily: font.body, fontSize: textSize.small, lineHeight: 1.5, color: colors.text }}>{children}</span>
    </div>
  );
}

function Code({ children }: { children: ReactNode }) {
  const { colors } = useTheme();
  return <code style={{ fontFamily: font.mono, fontSize: textSize.caption, color: colors.codeText, background: colors.codeBg, padding: `0 ${space.xs}px`, borderRadius: radius.xs }}>{children}</code>;
}
