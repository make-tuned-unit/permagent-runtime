import { useState, type CSSProperties } from 'react';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { HeroAgentPortrait } from './HeroAgentPortrait';
import { ForumPassport } from './ForumPassport';

type Props = {
  id: string; name: string; role: string; signature: string; color: string;
  status?: string; description: string; manner: string; voice: string; question: string;
  wire: string; sovereign: boolean;
  onChat: () => void; onSit: () => void; onBrief: () => void;
  onQuestion: () => void; onLearn: () => void;
};

/** A profile first; the longer operating and identity records remain on demand. */
export function AgentDeskProfile(props: Props) {
  const { colors } = useTheme();
  const [passportOpen, setPassportOpen] = useState(false);
  return <section className="agent-desk-profile" aria-label={`${props.name} profile`} style={{ '--agent-trim': props.color } as CSSProperties}>
    <div className="agent-desk-profile-heading">
      <div className="agent-desk-portrait-frame"><HeroAgentPortrait id={props.id} size="profile" label={props.signature} /></div>
      <div className="agent-desk-profile-identity">
        <span className="agent-desk-role">{props.sovereign ? 'Sovereign orchestrator' : props.role}</span>
        <h3>{props.name}</h3>
        <p className="agent-desk-reported" role="status"><span aria-hidden="true" />Reported: {props.status?.replace(/_/g, ' ') || 'not yet available'}</p>
      </div>
    </div>
    <div className="agent-desk-primary-actions" role="group" aria-label="Conversation and briefs">
      <Button colors={colors} variant="primary" flashSuccess={false} onClick={props.onChat}>Chat <span aria-hidden="true">↗</span></Button>
      {props.sovereign && <Button colors={colors} flashSuccess={false} aria-label={`Sit with ${props.name}`} onClick={props.onSit}>Sit together</Button>}
      <Button colors={colors} flashSuccess={false} onClick={props.onBrief}>Write a brief</Button>
    </div>
    <p className="agent-desk-chat-context">Chat continues your shared conversation.{!props.sovereign && ' Briefs include this role as context for your orchestrator.'}</p>
    <div className="agent-desk-question">
      <span className="eyebrow">START WITH A QUESTION</span>
      <p className="agent-desk-suggested-question">“{props.question}”</p>
      <Button colors={colors} variant="bare" flashSuccess={false} onClick={props.onQuestion}>Try a question for this role <span aria-hidden="true">↗</span></Button>
    </div>
    <details className="agent-desk-disclosure">
      <summary>Role & working style</summary>
      <p>{props.description}</p>
      <p>{props.manner}</p>
      <blockquote>{props.voice}</blockquote>
      <p>{props.wire === 'daemon'
        ? 'This agent has a daemon event source. Its presence alone does not confirm a connection or a running job.'
        : props.wire === 'sim'
          ? 'Ambient presence. Idle movement is simulated; it is not evidence of a running job.'
          : 'Static presence. This character has no live activity emitter.'}</p>
      <Button colors={colors} variant="bare" flashSuccess={false} onClick={props.onLearn}>How agents work ↗</Button>
    </details>
    {props.sovereign && <details className="agent-desk-disclosure" onToggle={event => setPassportOpen(event.currentTarget.open)}>
      <summary>Identity & passport <span>Optional</span></summary>
      {passportOpen && <ForumPassport />}
    </details>}
  </section>;
}
