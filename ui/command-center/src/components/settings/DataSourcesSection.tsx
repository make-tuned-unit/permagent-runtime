/**
 * Settings → Data sources.
 *
 * Browse one public-apis category at a time. Enabling a source makes it
 * callable immediately (suggested agents + the Orchestrator).
 */

import { useCallback, useEffect, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { font, radius, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Toggle } from '../common/Toggle';

interface CategoryView {
  name: string;
  count: number;
  suggestedAgents: string[];
}

interface CatalogEntry {
  slug: string;
  name: string;
  category: string;
  description: string;
  auth: string;
  https: boolean;
  cors: string;
  docsUrl: string;
  suggestedAgents: string[];
  enabled: boolean;
  keyPresent: boolean;
}

interface CatalogResponse {
  categories: CategoryView[];
  entries: CatalogEntry[];
  enabled: string[];
}

const AGENT_LABEL: Record<string, string> = {
  orchestrator: 'Orchestrator',
  financier: 'Financier',
  forecaster: 'Forecaster',
  librarian: 'Librarian',
  strix: 'The Guard',
  permagent: 'Permagent',
  reviewer: 'Reviewer',
};

function labelAgent(id: string): string {
  return AGENT_LABEL[id] ?? id;
}

export function DataSourcesSection() {
  const { colors } = useTheme();
  const [categories, setCategories] = useState<CategoryView[]>([]);
  const [category, setCategory] = useState('Finance');
  const [entries, setEntries] = useState<CatalogEntry[]>([]);
  const [enabled, setEnabled] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [keyDraft, setKeyDraft] = useState<Record<string, string>>({});

  const load = useCallback(async (cat: string) => {
    try {
      const data = await apiFetch<CatalogResponse>(
        `/api/public-apis/catalog?category=${encodeURIComponent(cat)}`,
      );
      setCategories(data.categories);
      setEntries(data.entries);
      setEnabled(data.enabled);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not load data sources');
    }
  }, []);

  useEffect(() => { void load(category); }, [category, load]);

  const toggle = async (slug: string, on: boolean) => {
    setBusy(slug);
    try {
      await apiFetch(`/api/public-apis/${encodeURIComponent(slug)}/enable`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ enabled: on }),
      });
      await load(category);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not update that source');
    } finally {
      setBusy(null);
    }
  };

  const saveKey = async (slug: string) => {
    const value = (keyDraft[slug] ?? '').trim();
    if (!value) return;
    setBusy(slug);
    try {
      await apiFetch(`/api/public-apis/${encodeURIComponent(slug)}/key`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ value }),
      });
      setKeyDraft((d) => ({ ...d, [slug]: '' }));
      await load(category);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not save the key');
    } finally {
      setBusy(null);
    }
  };

  const current = categories.find((c) => c.name === category);
  const visible = entries.slice(0, 24);

  return (
    <div data-testid="data-sources" style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <p style={{ ...type.small, color: colors.textMuted, margin: 0, lineHeight: 1.5 }}>
        From the public-apis catalog. Off until you turn a source on. Then it
        flows immediately to the suggested agents below — and the Orchestrator
        can call every enabled source, no restart.
        {enabled.length > 0 ? ` ${enabled.length} on.` : ''}
      </p>
      {error && (
        <div style={{ ...type.caption, color: colors.danger }}>{error}</div>
      )}
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }} role="tablist" aria-label="API categories">
        {categories.map((c) => {
          const on = c.name === category;
          return (
            <button
              key={c.name}
              type="button"
              role="tab"
              aria-selected={on}
              onClick={() => setCategory(c.name)}
              style={{
                ...type.micro,
                padding: '6px 10px',
                borderRadius: radius.sm,
                border: `1px solid ${on ? colors.cyan : colors.border}`,
                background: 'transparent',
                color: on ? colors.cyan : colors.text,
                cursor: 'pointer',
                fontFamily: font.body,
              }}
            >
              {c.name} · {c.count}
            </button>
          );
        })}
      </div>
      {current && (
        <div style={{ ...type.caption, color: colors.textMuted }}>
          Suggested agents: {current.suggestedAgents.map(labelAgent).join(', ')}
        </div>
      )}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {visible.map((e) => (
          <article
            key={e.slug}
            data-testid="data-source-row"
            data-slug={e.slug}
            style={{
              border: `1px solid ${colors.border}`,
              borderRadius: radius.md,
              padding: '12px 14px',
              background: colors.bgDeeper,
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
            }}
          >
            <div style={{ display: 'flex', gap: 12, alignItems: 'flex-start' }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{e.name}</div>
                <div style={{ ...type.caption, color: colors.textMuted, marginTop: 2 }}>{e.description}</div>
                <div style={{ ...type.caption, color: colors.textDim, marginTop: 4 }}>
                  Auth {e.auth || 'No'}
                  {e.https ? ' · HTTPS' : ''}
                  {' · '}
                  {e.suggestedAgents.map(labelAgent).join(', ')}
                </div>
              </div>
              <Toggle
                on={e.enabled}
                disabled={busy === e.slug}
                onChange={(v) => toggle(e.slug, v)}
                label={`Enable ${e.name}`}
              />
            </div>
            {e.enabled && e.auth.toLowerCase() === 'apikey' && (
              <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                <input
                  type="password"
                  autoComplete="off"
                  placeholder={e.keyPresent ? 'key saved — paste to replace' : 'API key'}
                  value={keyDraft[e.slug] ?? ''}
                  onChange={(ev) => setKeyDraft((d) => ({ ...d, [e.slug]: ev.target.value }))}
                  style={{
                    flex: 1, fontFamily: font.mono, fontSize: 11, color: colors.text,
                    background: colors.inputBg, border: `1px solid ${colors.border}`,
                    borderRadius: radius.sm, padding: '6px 8px', outline: 'none', minWidth: 0,
                  }}
                />
                <button
                  type="button"
                  disabled={busy === e.slug || !(keyDraft[e.slug] ?? '').trim()}
                  onClick={() => void saveKey(e.slug)}
                  style={{
                    ...type.micro,
                    padding: '6px 10px',
                    borderRadius: radius.sm,
                    border: `1px solid ${colors.cyan}`,
                    color: colors.cyan,
                    background: 'transparent',
                    cursor: 'pointer',
                    fontFamily: font.body,
                  }}
                >
                  Save key
                </button>
              </div>
            )}
          </article>
        ))}
        {entries.length > visible.length && (
          <div style={{ ...type.caption, color: colors.textMuted }}>
            Showing {visible.length} of {entries.length}. Pick a narrower need from the category chips.
          </div>
        )}
        {entries.length === 0 && (
          <div style={{ ...type.caption, color: colors.textMuted }}>No sources in this category.</div>
        )}
      </div>
    </div>
  );
}
