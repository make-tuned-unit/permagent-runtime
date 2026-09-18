/**
 * The Strategy lens — the five GTM pillars plus the brand kit.
 *
 * Split out of GrowView.tsx (R9). The lens section moved with its cards so the
 * "✦ Generate fills this in" promise and the card that honours it live in one
 * file; the view now names the lens and passes it a project.
 */

import { useState } from 'react';
import type { CSSProperties } from 'react';
import { font, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import type { IconType } from 'react-icons';
import { FiCalendar, FiEdit3, FiShare2, FiTarget, FiUsers, FiZap } from 'react-icons/fi';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';
import type { Project } from '../projects/types';
import { growAccent, growBare, growChip } from './growStyles';
import { FIELD_CLASS, growCard, growField, growLabel } from './growChrome';
import { CARD_INNER_R, CARD_PAD, CARD_R } from './growGeometry';
import {
  PILLARS,
  brandPrompt,
  readBrand,
  readStrategy,
  runAllPrompt,
  saveBrand,
  saveStrategy,
  type ProjectBrand,
  type SavedPillar,
} from './growStrategy';


/**
 * The lens itself. `send` is GrowView's one-click hand-off to the chat dock —
 * passed in rather than reached for, so this file has no opinion about how a
 * prompt reaches the agent.
 */
export function StrategyLens({
  active, colors, send, agentName,
}: {
  active: Project;
  colors: ThemeColors;
  send: (prompt: string) => void;
  agentName: string;
}) {
  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', margin: `0 0 ${space.xl}px` }}>
        <h3 style={{ ...growLabel(colors), margin: 0 }}>Go-to-market strategy</h3>
        <Tooltip content={`${agentName} researches every pillar and fills these cards with the results`} placement="left">
          <Button
            colors={colors}
            onClick={() => send(runAllPrompt(active.name))}
            style={{ ...growAccent(colors, `${space.sm}px ${space.xxl}px`), '--pa-btn-weight': 600, fontSize: textSize.caption } as CSSProperties}
          >✦ Generate</Button>
        </Tooltip>
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: space.xl }}>
        {PILLARS.map((pillar) => (
          <PillarCard
            key={pillar.key}
            pillarKey={pillar.key}
            label={pillar.label}
            hint={pillar.hint}
            colors={colors}
            saved={readStrategy(active, pillar.key)}
            onSave={(content) => saveStrategy(active.id, pillar.key, content)}
          />
        ))}
        <BrandCard
          colors={colors}
          brand={readBrand(active)}
          onAsk={() => send(brandPrompt(active.name))}
          onSave={(next) => saveBrand(active.id, next)}
          agentName={agentName}
        />
      </div>
    </section>
  );
}

// ── Strategy pillar card ─────────────────────────────────────────────────────
// The whole card is the interactive surface (mirrors DecisionsCard): clickable,
// keyboard-operable (Enter/Space), with hover + focus affordances. The "Ask
// Henry" chip is a visual cue, not a nested control.
// Feather-style icon per pillar — the card's identity at a glance.
/** Feather components, not path data: these six were hand-drawn copies of
 *  glyphs the library already ships (design-system ruling U2 §3.4 — one icon
 *  library, one ratified local set, nothing else). */
const PILLAR_ICONS: Record<string, IconType> = {
  audience: FiUsers,
  value: FiZap,
  positioning: FiTarget,
  channels: FiShare2,
  content: FiEdit3,
  workback: FiCalendar,
};

/** Strategy pillar card — display + edit only (#22). Generation is the single
 *  ✦ Generate button on the lens header; per-card Ask-Henry chips are gone.
 *  A saved pillar renders rich: summary, labeled points, stat chips. */
function PillarCard({
  pillarKey, label, hint, colors, saved, onSave,
}: {
  pillarKey: string;
  label: string;
  hint: string;
  colors: ThemeColors;
  /** Persisted strategy for this pillar (metadata_json.strategy), if any. */
  saved: SavedPillar | null;
  onSave: (content: string) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState(false);

  // Returns false on the swallowed failure so the Button contract can never
  // tick a save that did not land.
  const commit = async () => {
    const content = draft.trim();
    if (!content) { setEditing(false); return true; }
    setSaving(true);
    try {
      await onSave(content); // project_changed → projectsRev → cards refresh
      setEditing(false);
      return true;
    } catch {
      setSaveError(true);
      return false;
    } finally {
      setSaving(false);
    }
  };

  // Opaque, no blur (D1). This is a card — a bordered box with a label, a body
  // and a save state, sitting in the page's flow. Cards are content, and Apple
  // keeps glass off the content layer. Elevation comes from the fill and the
  // hairline, which is where it should come from.
  const shell: CSSProperties = {
    ...growCard(colors, { r: CARD_R, pad: CARD_PAD, accent: !!saved }),
    display: 'flex', flexDirection: 'column', gap: space.lg, minHeight: 120,
  };

  if (editing) {
    return (
      <div style={shell}>
        <div style={{ fontFamily: font.body, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>{label}</div>
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          autoFocus
          rows={6}
          style={{
            width: '100%', resize: 'vertical', fontSize: textSize.caption, lineHeight: 1.5,
            fontFamily: font.body, color: colors.text, background: 'transparent',
            border: `1px solid ${colors.border}`, borderRadius: CARD_INNER_R, padding: space.md,
            outline: 'none',
          }}
        />
        {saveError && <span style={{ fontSize: textSize.micro, color: colors.danger }}>Couldn't save — try again.</span>}
        <div style={{ display: 'flex', gap: space.md }}>
          <Button colors={colors} onClick={() => commit()} disabled={saving} style={{ ...growAccent(colors), '--pa-btn-weight': 600 } as CSSProperties}>
            {saving ? 'Saving…' : 'Save'}
          </Button>
          <Button colors={colors} variant="bare" onClick={() => setEditing(false)} style={growBare(colors)}>
            Cancel
          </Button>
        </div>
      </div>
    );
  }

  const PillarIcon = PILLAR_ICONS[pillarKey] ?? PILLAR_ICONS.value;

  return (
    <div style={shell}>
      <div style={{ display: 'flex', alignItems: 'center', gap: space.md }}>
        <PillarIcon size={15} style={{ flexShrink: 0 }} color={saved ? colors.cyan : colors.textMuted} />
        <span style={{ fontFamily: font.body, fontSize: textSize.body, fontWeight: 600, color: colors.text, flex: 1 }}>{label}</span>
        {saved && (
          <Tooltip content={saved.updated_at ? `Saved ${new Date(saved.updated_at).toLocaleString()}` : 'Edit'}>
            <Button
              colors={colors}
              variant="bare"
              onClick={() => { setDraft(saved.content); setSaveError(false); setEditing(true); }}
              style={{ ...growBare(colors), fontSize: textSize.micro }}
            >Edit</Button>
          </Tooltip>
        )}
      </div>

      {saved ? (
        <>
          <div style={{
            fontSize: textSize.caption, color: colors.text, lineHeight: 1.55,
            whiteSpace: 'pre-wrap', overflowWrap: 'break-word',
          }}>{saved.content}</div>

          {saved.points && saved.points.length > 0 && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: space.xs }}>
              {saved.points.map((pt, i) => (
                <div key={i} style={{ display: 'flex', gap: space.sm, fontSize: textSize.caption, lineHeight: 1.45 }}>
                  <span style={{ color: colors.cyan, flexShrink: 0 }}>▸</span>
                  <span style={{ color: colors.textMuted, overflowWrap: 'break-word', minWidth: 0 }}>
                    <span style={{ color: colors.text, fontWeight: 600 }}>{pt.label}</span>
                    {' — '}{pt.detail}
                  </span>
                </div>
              ))}
            </div>
          )}

          {saved.metrics && saved.metrics.length > 0 && (
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.sm, marginTop: 'auto' }}>
              {saved.metrics.map((m, i) => (
                <Tooltip content={m.label}>
                  <span tabIndex={0} style={{ outline: 'none' }}>
                    <span key={i} style={{
                      // Chips must never exceed the card: wrap long label·value
                      // pairs inside the pill instead of bleeding across the grid.
                      fontSize: textSize.micro, fontFamily: font.mono, lineHeight: 1.4,
                      maxWidth: '100%', overflowWrap: 'anywhere',
                      color: colors.cyan, background: colors.cyanSoft,
                      border: `1px solid ${colors.borderHi}`, borderRadius: CARD_INNER_R,
                      padding: `${space.xs}px ${space.md}px`,
                    }}>
                      <span style={{ color: colors.textMuted }}>{m.label} · </span>{m.value}
                    </span>
                  </span>
                </Tooltip>
              ))}
            </div>
          )}
        </>
      ) : (
        <>
          <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5, flex: 1 }}>{hint}</div>
          <span style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.body }}>
            ✦ Generate fills this in
          </span>
        </>
      )}
    </div>
  );
}

function BrandCard({
  colors, brand, onAsk, onSave, agentName,
}: {
  colors: ThemeColors;
  brand: ProjectBrand;
  onAsk: () => void;
  onSave: (brand: ProjectBrand) => Promise<void>;
  agentName: string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(brand);
  const [saving, setSaving] = useState(false);
  const filled = !!(brand.voice || brand.origin || brand.bg);
  const shell: CSSProperties = {
    ...growCard(colors, { r: CARD_R, pad: CARD_PAD, accent: filled }),
    display: 'flex', flexDirection: 'column', gap: space.lg, minHeight: 120,
  };
  const field: CSSProperties = { ...growField(colors), width: '100%', borderRadius: CARD_INNER_R };
  if (editing) {
    return (
      <div style={shell}>
        <div style={{ fontFamily: font.body, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>Brand</div>
        <textarea value={draft.voice} onChange={(e) => setDraft({ ...draft, voice: e.target.value })} placeholder="Voice" rows={3} className={FIELD_CLASS} style={field} />
        <textarea value={draft.origin} onChange={(e) => setDraft({ ...draft, origin: e.target.value })} placeholder="Why this was built" rows={3} className={FIELD_CLASS} style={field} />
        <div style={{ display: 'flex', gap: space.sm }}>
          <input value={draft.bg} onChange={(e) => setDraft({ ...draft, bg: e.target.value })} placeholder="#bg" aria-label="Background hex" className={FIELD_CLASS} style={field} />
          <input value={draft.fg} onChange={(e) => setDraft({ ...draft, fg: e.target.value })} placeholder="#fg" aria-label="Foreground hex" className={FIELD_CLASS} style={field} />
          <input value={draft.accent} onChange={(e) => setDraft({ ...draft, accent: e.target.value })} placeholder="#accent" aria-label="Accent hex" className={FIELD_CLASS} style={field} />
        </div>
        <div style={{ display: 'flex', gap: space.md }}>
          {/* Returning the save promise instead of dropping it on the floor is
              what buys the spinner and the tick — the round trip is the thing
              the user is waiting on. */}
          <Button
            colors={colors}
            type="button"
            disabled={saving}
            onClick={() => {
              setSaving(true);
              return onSave(draft)
                .then(() => { setEditing(false); return true; })
                .finally(() => setSaving(false));
            }}
            style={{ ...growAccent(colors), '--pa-btn-weight': 600 } as CSSProperties}
          >{saving ? 'Saving…' : 'Save'}</Button>
          <Button
            colors={colors}
            variant="bare"
            type="button"
            onClick={() => setEditing(false)}
            style={growBare(colors)}
          >Cancel</Button>
        </div>
      </div>
    );
  }
  return (
    <div style={shell}>
      <div style={{ display: 'flex', alignItems: 'center', gap: space.md }}>
        <div style={{ fontFamily: font.body, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>Brand</div>
        <div style={{ flex: 1 }} />
        <Button colors={colors} type="button" onClick={onAsk} style={growChip()}>Ask {agentName}</Button>
        <Button colors={colors} type="button" onClick={() => { setDraft(brand); setEditing(true); }} style={growChip()}>Edit</Button>
      </div>
      {filled ? (
        <>
          {brand.voice && <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}>{brand.voice}</div>}
          {brand.origin && <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}>{brand.origin}</div>}
          <div style={{ display: 'flex', gap: space.sm }}>
            {[['bg', brand.bg], ['fg', brand.fg], ['accent', brand.accent]].filter(([, v]) => v).map(([k, v]) => (
              <span key={k} style={{ fontSize: textSize.micro, fontFamily: font.mono, color: colors.textDim }}>{k} {v}</span>
            ))}
          </div>
        </>
      ) : (
        <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}>
          Voice, palette, and why this project was built. Empty until you save a kit for this project — nothing is shared across projects.
        </div>
      )}
    </div>
  );
}
