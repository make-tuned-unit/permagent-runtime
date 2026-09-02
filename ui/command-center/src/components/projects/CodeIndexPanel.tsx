/**
 * CodeIndexPanel — the "Codebase" action on a project's Overview.
 *
 * One button that indexes the project's codebase into the Brain: it POSTs to
 * `/api/projects/{id}/index-code`, which runs the analyze extension's
 * tree-sitter structure pass over the project's root_path and stores the
 * resulting code map as a durable, project-scoped memory (description-less, so
 * the Librarian enriches it) — the same persistence pipeline a dropped document
 * or a written note rides. Code was the one artifact class that stayed
 * ephemeral; this makes it recallable and described like the rest (#471).
 *
 * Shown only when the project has a root_path (nothing to index otherwise).
 * Observability (per the #568 empty-body lesson): the backend answers a failure
 * with a plain-text reason, surfaced inline here rather than a silent catch.
 * Styled strictly with the shared Panel shell + theme tokens.
 */

import { useState, type CSSProperties } from 'react';
import { FiExternalLink } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { projectMemoryPreview, type BrainMemoryTarget } from '../brain/brainMemoryFocus';
import { font, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Panel } from './Panel';
import type { Project } from './types';

type Result = { files: number; memoryKey: string };

export function CodeIndexPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const [indexing, setIndexing] = useState(false);
  const [result, setResult] = useState<Result | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [viewing, setViewing] = useState(false);
  const focusBrainMemory = useCommandCenter(s => s.focusBrainMemory);

  // Nothing to index without a codebase location.
  if (!project.rootPath) return null;

  // Resolves `false` on failure: this helper surfaces its own error inline, so
  // the button must not tick over an index run that did not happen.
  const index = async () => {
    if (indexing) return false;
    setError(null);
    setIndexing(true);
    try {
      const res = await api.indexProjectCode(project.id);
      setResult({ files: res.files, memoryKey: res.memoryKey });
      return true;
    } catch (e) {
      setError(`Couldn't index code: ${(e as Error).message || 'request failed'}`);
      return false;
    } finally {
      setIndexing(false);
    }
  };

  // Close the loop: jump from the code map's key to where it lives in the Brain.
  // Resolve the key to its live memory via the same associated-memories endpoint
  // the Memories panel uses (the code map is written description-less, so it
  // isn't in the graph's default view until the Librarian enriches it — the
  // resolved content is the preview that guarantees the Brain renders it). Falls
  // back to a best-effort focus by key if the lookup can't find/reach it.
  const viewInBrain = async () => {
    if (!result || viewing) return false;
    setViewing(true);
    let target: BrainMemoryTarget = { key: result.memoryKey };
    try {
      const mems = await api.listProjectMemories(project.id);
      const hit = mems.find(m => m.key === result.memoryKey);
      if (hit) target = { id: hit.id, key: hit.key, preview: projectMemoryPreview(hit) };
    } catch {
      // best-effort: fall back to focusing by key alone
    }
    setViewing(false);
    focusBrainMemory(target);
    return true;
  };

  return (
    <Panel title="Codebase">
      <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.55, marginBottom: 10 }}>
        Parse this project's code into your Brain — its directory structure and
        symbols become recallable and described, the same way its documents and
        notes are.
      </div>
      <Button
        colors={colors}
        variant="ghostOn"
        onClick={index}
        disabled={indexing}
        style={{
          '--pa-btn-bg': colors.cyanSoft,
          '--pa-btn-border': colors.borderHi,
          '--pa-btn-pad': '6px 14px',
          '--pa-btn-radius': '7px',
          '--pa-btn-weight': 600,
          fontFamily: font.body,
          fontSize: textSize.caption,
        } as CSSProperties}
      >
        {indexing ? 'Indexing…' : result ? 'Re-index code' : "Index this project's code"}
      </Button>

      {error && (
        <div style={{ fontSize: textSize.micro, color: colors.danger, marginTop: 8 }}>{error}</div>
      )}

      {result && !error && (
        <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 8, lineHeight: 1.5 }}>
          <span style={{ color: colors.success, fontWeight: 600 }}>
            Indexed {result.files} file{result.files !== 1 ? 's' : ''}
          </span>{' '}
          into your Brain, scoped to this project.
          {/* The code map's key, now a link back into the Brain (was inert text). */}
          <Button
            colors={colors}
            variant="bare"
            onClick={viewInBrain}
            disabled={viewing}
            title="View this code map in your Brain"
            // `Button` wraps children in a `.pa-btn__label` span, which would
            // put the truncating key in a box of its own and kill the ellipsis.
            // `display: contents` dissolves the wrapper so the key and the icon
            // are the button's own flex children, exactly as before.
                        style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-weight': 'inherit',
              marginTop: 3,
              maxWidth: '100%',
              gap: 5,
              fontFamily: font.mono,
              fontSize: 10,
            } as CSSProperties}
          >
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {result.memoryKey}
            </span>
            <FiExternalLink size={10} style={{ flexShrink: 0 }} />
          </Button>
        </div>
      )}
    </Panel>
  );
}
