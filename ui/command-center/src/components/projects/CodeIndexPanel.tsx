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

import { useState } from 'react';
import { api } from '../../lib/api';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Panel } from './Panel';
import type { Project } from './types';

type Result = { files: number; memoryKey: string };

export function CodeIndexPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const [indexing, setIndexing] = useState(false);
  const [result, setResult] = useState<Result | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Nothing to index without a codebase location.
  if (!project.rootPath) return null;

  const index = async () => {
    if (indexing) return;
    setError(null);
    setIndexing(true);
    try {
      const res = await api.indexProjectCode(project.id);
      setResult({ files: res.files, memoryKey: res.memoryKey });
    } catch (e) {
      setError(`Couldn't index code: ${(e as Error).message || 'request failed'}`);
    } finally {
      setIndexing(false);
    }
  };

  return (
    <Panel title="Codebase">
      <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.55, marginBottom: 10 }}>
        Parse this project's code into your Brain — its directory structure and
        symbols become recallable and described, the same way its documents and
        notes are.
      </div>
      <button
        onClick={index}
        disabled={indexing}
        style={{
          fontSize: 12, fontWeight: 600, padding: '6px 14px', borderRadius: 7,
          cursor: indexing ? 'default' : 'pointer',
          opacity: indexing ? 0.5 : 1,
          background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
          color: colors.cyan, fontFamily: font.body,
        }}
      >
        {indexing ? 'Indexing…' : result ? 'Re-index code' : "Index this project's code"}
      </button>

      {error && (
        <div style={{ fontSize: 11, color: colors.danger, marginTop: 8 }}>{error}</div>
      )}

      {result && !error && (
        <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 8, lineHeight: 1.5 }}>
          <span style={{ color: colors.success, fontWeight: 600 }}>
            Indexed {result.files} file{result.files !== 1 ? 's' : ''}
          </span>{' '}
          into your Brain, scoped to this project.
          <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, marginTop: 3 }}>
            {result.memoryKey}
          </div>
        </div>
      )}
    </Panel>
  );
}
