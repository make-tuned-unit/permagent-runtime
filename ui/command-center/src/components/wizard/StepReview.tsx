import { useEffect, useId, useState, type ReactNode } from 'react';
import { font, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { api } from '../../lib/api';
import { SEARCH_PROVIDERS } from '../../lib/searchProviders';
import { FieldLabel, GhostLink, StatusLine, Textarea } from './atoms';
import { StepFrame } from './StepFrame';
import { LOCAL_RUNTIME_PROVIDER } from './providerChoices';

export interface ReviewFacts {
  model: { connected: boolean; text: string };
  folders: string;
  search: string;
  memory: string;
}

/** Turn what the backend holds into the review rows. Pure, for tests. */
export function summarize(input: {
  config: Record<string, unknown> | undefined;
  providers: Array<{ name: string; is_configured: boolean; metadata: { display_name: string } }>;
  searchSaved: string[];
  librarian: { enabled: boolean; model: string } | null;
}): ReviewFacts {
  const cfg = input.config ?? {};
  const providerName = typeof cfg.GOOSE_PROVIDER === 'string' ? cfg.GOOSE_PROVIDER : '';
  const modelName = typeof cfg.GOOSE_MODEL === 'string' ? cfg.GOOSE_MODEL : '';
  const entry = input.providers.find(p => p.name === providerName);
  const connected = !!entry && entry.is_configured;
  const where = providerName === LOCAL_RUNTIME_PROVIDER ? 'On this Mac' : entry?.metadata.display_name || providerName;
  const roots = Array.isArray(cfg.dev_roots) ? cfg.dev_roots.filter(r => typeof r === 'string') : [];
  return {
    model: connected
      ? { connected, text: [where, modelName].filter(Boolean).join(' · ') }
      : { connected, text: 'Not connected' },
    folders: roots.length === 0 ? 'None' : roots.length === 1 ? String(roots[0]) : `${roots.length} folders`,
    search: input.searchSaved.length ? input.searchSaved.join(', ') : 'Not set up',
    memory: input.librarian?.enabled ? `On · ${input.librarian.model}` : 'Off',
  };
}

interface Props {
  agentName: string;
  toneLabel: string;
  firstMessage: string;
  onFirstMessage: (v: string) => void;
  /** Jump to a step to change it; the shell brings the person back here. */
  onEdit: (step: number) => void;
  steps: { model: number; agent: number; folders: number; extras: number };
  onStart: () => unknown;
  saving: boolean;
}

/**
 * The end of setup: what is actually configured (re-read from the backend, so
 * a resumed or half-finished run shows the truth), a Change link per row, an
 * optional first message, and Start.
 */
export function StepReview({ agentName, toneLabel, firstMessage, onFirstMessage, onEdit, steps, onStart, saving }: Props) {
  const msgId = useId();
  const [facts, setFacts] = useState<ReviewFacts | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const [config, providers, librarian, searchSaved] = await Promise.all([
          api.getConfig(),
          api.getProviders(),
          api.getLibrarianSchedule().catch(() => null),
          Promise.all(SEARCH_PROVIDERS.map(async p => {
            try { return (await api.readSecretConfig(p.keyName))?.maskedValue ? p.displayName : null; } catch { return null; }
          })).then(xs => xs.filter((x): x is string => !!x)),
        ]);
        if (alive) setFacts(summarize({ config: config.config, providers, searchSaved, librarian }));
      } catch (e) {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => { alive = false; };
  }, []);

  return (
    <StepFrame
      title="Ready when you are"
      why="Here's what's set up. Change anything now, or later in Settings."
      primary={{ label: 'Start Permagent', pending: saving, pendingLabel: 'Starting…' }}
      onPrimary={onStart}
    >
      {error && <StatusLine tone="warn">Couldn't read back every setting ({error}). Your choices were saved as you made them.</StatusLine>}
      <dl style={{ margin: 0, display: 'flex', flexDirection: 'column' }}>
        <Row label="Model" value={facts ? facts.model.text : '…'} warn={!!facts && !facts.model.connected} onChange={() => onEdit(steps.model)} />
        <Row label="Agent" value={`${agentName} · ${toneLabel}`} onChange={() => onEdit(steps.agent)} />
        <Row label="Code folders" value={facts?.folders ?? '…'} onChange={() => onEdit(steps.folders)} />
        <Row label="Web search" value={facts?.search ?? '…'} onChange={() => onEdit(steps.extras)} />
        <Row label="Memory model" value={facts?.memory ?? '…'} onChange={() => onEdit(steps.extras)} last />
      </dl>

      {facts && !facts.model.connected && (
        <StatusLine tone="warn">No model is connected, so {agentName} can't reply yet. Add one any time in Settings → Models.</StatusLine>
      )}

      <div>
        <FieldLabel htmlFor={msgId}>First message (optional)</FieldLabel>
        <Textarea id={msgId} value={firstMessage} onChange={onFirstMessage} rows={2}
          placeholder={`Anything you want to start with? It will be waiting in the chat for you to send.`} />
      </div>
    </StepFrame>
  );
}

function Row({ label, value, onChange, warn, last }: {
  label: string; value: ReactNode; onChange: () => void; warn?: boolean; last?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <div style={{
      display: 'grid', gridTemplateColumns: '128px 1fr auto', alignItems: 'center', gap: space.xl,
      padding: `${space.md}px 0`, borderBottom: last ? 'none' : `1px solid ${colors.border}`,
    }}>
      <dt style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted }}>{label}</dt>
      <dd style={{ margin: 0, fontFamily: font.body, fontSize: textSize.body, color: warn ? colors.warning : colors.text, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{value}</dd>
      <GhostLink onClick={onChange} ariaLabel={`Change ${label.toLowerCase()}`}>Change</GhostLink>
    </div>
  );
}
