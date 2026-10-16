import { useCallback, useEffect, useRef, useState } from 'react';
import { useTheme } from '../../styles/useTheme';
import { radius, space, textSize } from '../../styles/tokens';
import { Button } from '../common/Button';
import { useCommandCenter } from '../../lib/store';
import { detectProjectStack, type DetectedStack as Detection } from './stackEntries';
import type { Project } from './types';

/** Repository declarations are evidence, never proof of a live deployment. */
export function DetectedStack({ project }: { project: Project }) {
  const { colors } = useTheme();
  const generation = useRef(0);
  const [result, setResult] = useState<Detection | null>(null);
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');
  const load = useCallback(async () => {
    const request = ++generation.current;
    setStatus('loading');
    try {
      const next = await detectProjectStack(project.id);
      if (!Array.isArray(next.entries) || typeof next.scanned_files !== 'number') throw new Error('Invalid discovery response');
      if (generation.current !== request) return;
      setResult(next);
      setStatus('ready');
    } catch {
      if (generation.current === request) setStatus('error');
    }
  }, [project.id, project.rootPath]);
  useEffect(() => {
    setResult(null);
    if (project.rootPath) void load();
    return () => { generation.current++; };
  }, [load, project.rootPath]);
  return <section aria-label="Stack detected from code" style={{ marginBottom: space.xl, paddingBottom: space.lg, borderBottom: `1px solid ${colors.border}` }}>
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.md }}>
      <strong style={{ fontSize: textSize.caption }}>Detected from code</strong>
      {project.rootPath && <Button colors={colors} variant="bare" disabled={status === 'loading'} onClick={() => void load()}>
        {status === 'error' ? 'Retry discovery' : 'Refresh discovery'}
      </Button>}
    </div>
    {!project.rootPath ? <div style={{ fontSize: textSize.caption, color: colors.textMuted }}>
      <p>Connect a code folder to discover this project’s stack automatically.</p>
      <Button colors={colors} onClick={() => {
        const state = useCommandCenter.getState();
        const request = `Help me connect the project "${project.name}" to its local code folder. Use its existing context to find the folder, ask me if the match is uncertain, and inspect its stack.`;
        state.setChatDraft({ text: [state.chatDraft.text, request].filter(Boolean).join('\n\n') });
        state.openChatDock();
      }}>Connect with agent</Button>
    </div>
      : status === 'loading' ? <p role="status" style={{ fontSize: textSize.caption, color: colors.textMuted }}>Reading project manifests…</p>
      : status === 'error' ? <p role="alert" style={{ fontSize: textSize.caption, color: colors.warning }}>Couldn’t read the project’s stack. Check that its code folder is available and try again.</p>
      : result && <>
        <p style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
          {result.scanned_files} manifest{result.scanned_files === 1 ? '' : 's'} read. These are code dependencies and configuration; connected accounts may differ.
        </p>
        {result.incomplete && <p role="status" style={{ fontSize: textSize.micro, color: colors.warning }}>Partial scan — some files were unreadable or exceeded discovery limits.</p>}
        {result.entries.length === 0 && <p style={{ fontSize: textSize.caption, color: colors.textMuted }}>No supported stack declarations found{result.incomplete ? ' in the files read' : ''}.</p>}
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.sm }}>
          {result.entries.map(entry => <span key={`${entry.category}-${entry.service_name}`} style={{ padding: `${space.xs}px ${space.md}px`, borderRadius: radius.md, background: colors.fillSubtle, fontSize: textSize.caption }}>{entry.service_name}</span>)}
        </div>
        {result.entries.length > 0 && <details style={{ marginTop: space.md }}>
          <summary style={{ cursor: 'pointer', fontSize: textSize.caption, color: colors.textMuted }}>View source files</summary>
          <dl style={{ fontSize: textSize.micro, lineHeight: 1.6 }}>
            {result.entries.map(entry => <div key={`${entry.category}-${entry.service_name}`} style={{ marginTop: space.sm }}>
              <dt>{entry.service_name}</dt><dd style={{ margin: 0, color: colors.textDim, overflowWrap: 'anywhere' }}>{entry.evidence.join(', ')}</dd>
            </div>)}
          </dl>
        </details>}
      </>}
  </section>;
}
