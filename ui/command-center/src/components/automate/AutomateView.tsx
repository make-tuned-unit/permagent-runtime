import { useEffect, useState, useMemo } from 'react';
import { color, font, ease, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';
import type { SkillState } from '../../lib/store';
import { SkillDetailPanel } from '../skills/SkillDetailPanel';

type Tab = 'skills' | 'builder';

export function AutomateView() {
  const skills = useCommandCenter(s => s.skills);
  const skillsLoading = useCommandCenter(s => s.skillsLoading);
  const loadSkills = useCommandCenter(s => s.loadSkills);
  const selectedSkillId = useCommandCenter(s => s.selectedSkillId);
  const setSelectedSkillId = useCommandCenter(s => s.setSelectedSkillId);

  const [search, setSearch] = useState('');
  const [tab, setTab] = useState<Tab>('skills');

  useEffect(() => { loadSkills(); }, [loadSkills]);

  const filtered = useMemo(() => {
    if (!search.trim()) return skills;
    const q = search.toLowerCase();
    return skills.filter(s =>
      s.name.toLowerCase().includes(q) ||
      (s.description && s.description.toLowerCase().includes(q))
    );
  }, [skills, search]);

  const selectedSkill = selectedSkillId ? skills.find(s => s.id === selectedSkillId) ?? null : null;
  const { gradient } = useTheme();

  return (
    <div style={{
      width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
      background: gradient.workspace,
      color: color.text, fontFamily: font.body,
    }}>
      {/* Header */}
      <div style={{
        padding: '20px 32px 16px', flexShrink: 0,
        borderBottom: `1px solid ${color.border}`,
      }}>
        <div style={{ fontFamily: font.display, fontSize: 20, fontWeight: 600 }}>Automate</div>
        <div style={{ fontSize: 12, color: color.textMuted, marginTop: 4 }}>
          Skills and automations that extend your agent's capabilities.
        </div>

        {/* Tabs */}
        <div style={{ display: 'flex', gap: 0, marginTop: 16, borderBottom: `1px solid ${color.border}`, marginBottom: -1 }}>
          {([['skills', 'Skills Library'], ['builder', 'Automation Builder']] as const).map(([id, label]) => (
            <button key={id} onClick={() => setTab(id)} style={{
              padding: '8px 16px', fontFamily: font.body, fontSize: 12, fontWeight: 600,
              color: tab === id ? color.cyan : color.textMuted,
              background: 'transparent', border: 'none', cursor: 'pointer',
              borderBottom: tab === id ? `2px solid ${color.cyan}` : '2px solid transparent',
              transition: `all 150ms ${ease.out}`,
            }}>{label}</button>
          ))}
        </div>

        {/* Search (skills tab only) */}
        {tab === 'skills' && <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginTop: 16 }}>
          <div style={{ position: 'relative', flex: 1, maxWidth: 360 }}>
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke={color.textDim} strokeWidth={2} strokeLinecap="round"
              style={{ position: 'absolute', left: 10, top: '50%', transform: 'translateY(-50%)' }}>
              <circle cx="11" cy="11" r="7" /><path d="M16 16l5 5" />
            </svg>
            <input
              value={search}
              onChange={e => setSearch(e.target.value)}
              placeholder="Search skills..."
              style={{
                width: '100%', fontFamily: font.body, fontSize: 13, color: color.text,
                background: 'rgba(20,28,48,0.4)',
                border: `1px solid ${color.border}`, borderRadius: radius.md,
                padding: '8px 12px 8px 32px', outline: 'none',
              }}
              onFocus={e => e.target.style.borderColor = 'rgba(0,213,255,0.18)'}
              onBlur={e => e.target.style.borderColor = 'rgba(255,255,255,0.07)'}
            />
          </div>
          <div style={{
            fontSize: 11, fontWeight: 600, letterSpacing: '0.08em',
            textTransform: 'uppercase', color: color.textDim,
          }}>
            {filtered.length} skill{filtered.length !== 1 ? 's' : ''}
          </div>
        </div>}
      </div>

      {/* Content */}
      {tab === 'builder' && <BuilderPlaceholder />}
      {tab === 'skills' && <div style={{ flex: 1, minHeight: 0, display: 'flex', overflow: 'hidden' }}>
        {/* Skills list */}
        <div style={{
          flex: selectedSkill ? '0 0 50%' : '1',
          overflowY: 'auto', padding: '20px 32px',
          borderRight: selectedSkill ? `1px solid ${color.border}` : 'none',
        }}>
          {skillsLoading ? (
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: 120, color: color.textDim, fontSize: 12 }}>
              Loading skills...
            </div>
          ) : filtered.length === 0 ? (
            <EmptyState hasSearch={!!search.trim()} search={search} />
          ) : (
            <div style={{ display: 'grid', gridTemplateColumns: selectedSkill ? '1fr' : 'repeat(auto-fill, minmax(300px, 1fr))', gap: 12 }}>
              {filtered.map(skill => (
                <SkillCardStyled
                  key={skill.id}
                  skill={skill}
                  isSelected={skill.id === selectedSkillId}
                  onSelect={setSelectedSkillId}
                />
              ))}
            </div>
          )}
        </div>

        {/* Detail panel */}
        {selectedSkill && (
          <div style={{ flex: '0 0 50%', overflow: 'hidden' }}>
            <SkillDetailPanel skill={selectedSkill} />
          </div>
        )}
      </div>}
    </div>
  );
}

function BuilderPlaceholder() {
  return (
    <div style={{
      flex: 1, display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center',
      gap: 16, padding: 32, textAlign: 'center',
    }}>
      <div style={{
        width: 64, height: 64, borderRadius: 16,
        background: 'rgba(141,68,174,0.08)',
        border: `1px solid rgba(141,68,174,0.20)`,
        display: 'grid', placeItems: 'center',
      }}>
        <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke={color.purple} strokeWidth={1.5} strokeLinecap="round" strokeOpacity={0.6}>
          <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
        </svg>
      </div>
      <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 600, color: color.text }}>
        Automation Builder
      </div>
      <div style={{ fontSize: 13, color: color.textMuted, maxWidth: 400, lineHeight: 1.6 }}>
        Create custom automations by chaining tools, triggers, and conditions.
        Define workflows that your agent executes autonomously.
      </div>
      <div style={{
        fontSize: 11, color: color.textDim, fontFamily: font.mono,
        padding: '6px 14px', borderRadius: radius.pill,
        background: 'rgba(141,68,174,0.06)',
        border: `1px solid rgba(141,68,174,0.15)`,
      }}>
        Coming soon
      </div>
    </div>
  );
}

function SkillCardStyled({ skill, isSelected, onSelect }: {
  skill: SkillState; isSelected: boolean; onSelect: (id: string | null) => void;
}) {
  return (
    <button
      onClick={() => onSelect(isSelected ? null : skill.id)}
      style={{
        display: 'flex', flexDirection: 'column', gap: 8,
        padding: 16, borderRadius: radius.md, textAlign: 'left',
        background: isSelected ? 'rgba(0,213,255,0.08)' : 'rgba(20,28,48,0.5)',
        border: `1px solid ${isSelected ? color.borderHi : color.border}`,
        cursor: 'pointer', width: '100%',
        transition: 'all 150ms ease',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <div style={{
          width: 28, height: 28, borderRadius: 8,
          background: 'rgba(0,213,255,0.10)',
          border: `1px solid ${color.borderHi}`,
          display: 'grid', placeItems: 'center',
          flexShrink: 0,
        }}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={color.cyan} strokeWidth={2} strokeLinecap="round">
            <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
          </svg>
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{
            fontFamily: font.body, fontSize: 13, fontWeight: 600, color: color.text,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>{skill.name}</div>
          {skill.trigger_type && (
            <div style={{ fontFamily: font.mono, fontSize: 10, color: color.textDim, marginTop: 2 }}>
              {skill.trigger_type}
            </div>
          )}
        </div>
      </div>
      {skill.description && (
        <div style={{
          fontSize: 12, color: color.textMuted, lineHeight: 1.5,
          overflow: 'hidden', textOverflow: 'ellipsis',
          display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical',
        } as React.CSSProperties}>{skill.description}</div>
      )}
    </button>
  );
}

function EmptyState({ hasSearch, search }: { hasSearch: boolean; search: string }) {
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
      height: '100%', minHeight: 200, gap: 12, textAlign: 'center',
    }}>
      <div style={{
        width: 48, height: 48, borderRadius: 12,
        background: 'rgba(0,213,255,0.06)',
        display: 'grid', placeItems: 'center',
      }}>
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke={color.cyan} strokeWidth={1.5} strokeLinecap="round" strokeOpacity={0.4}>
          <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
        </svg>
      </div>
      {hasSearch ? (
        <div style={{ fontSize: 13, color: color.textMuted }}>No skills match "{search}"</div>
      ) : (
        <>
          <div style={{ fontSize: 14, fontWeight: 500, color: color.textMuted }}>No skills yet</div>
          <div style={{ fontSize: 12, color: color.textDim, maxWidth: 280 }}>
            Skills are created automatically when your agent detects repeated patterns in your workflows.
          </div>
        </>
      )}
    </div>
  );
}
