import { useEffect, useState, useMemo, type CSSProperties } from 'react';
import { FiLoader, FiZap, FiSearch, FiGrid, FiList, FiX } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { font, radius, space } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { SkillCard } from './SkillCard';
import { SkillDetailPanel } from './SkillDetailPanel';

import { Tooltip } from '../common/Tooltip';
/**
 * Full Skills Library. Reused as a workspace tool (no chrome) and as the
 * `activePanel:'skills'` overlay — when hosted as an overlay, `onClose` is
 * provided so the surface offers a Close button + Escape to dismiss back to the
 * workspace (mirrors InboxPanel; a workspace host passes nothing and shows no
 * Close affordance).
 */
export function SkillsPanel({ onClose }: { onClose?: () => void } = {}) {
  const { colors } = useTheme();
  const skills = useCommandCenter(s => s.skills);
  const skillsLoading = useCommandCenter(s => s.skillsLoading);
  const skillsError = useCommandCenter(s => s.skillsError);
  const loadSkills = useCommandCenter(s => s.loadSkills);
  const selectedSkillId = useCommandCenter(s => s.selectedSkillId);
  const setSelectedSkillId = useCommandCenter(s => s.setSelectedSkillId);

  const [search, setSearch] = useState('');
  const [viewMode, setViewMode] = useState<'list' | 'grid'>('list');

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

  // Overlay dismissal — Escape closes back to the workspace, but only when
  // hosted as an overlay (onClose provided).
  useEffect(() => {
    if (!onClose) return;
    const h = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); onClose(); } };
    window.addEventListener('keydown', h);
    return () => window.removeEventListener('keydown', h);
  }, [onClose]);

  const filtered = useMemo(() => {
    if (!search.trim()) return skills;
    const q = search.toLowerCase();
    return skills.filter(s =>
      s.name.toLowerCase().includes(q) ||
      (s.description && s.description.toLowerCase().includes(q))
    );
  }, [skills, search]);

  const selectedSkill = selectedSkillId
    ? skills.find(s => s.id === selectedSkillId) ?? null
    : null;

  // Both header affordances rest muted and come up to full on hover — what the
  // pair of mouse handlers here used to do by hand, expressed so `:active` and
  // the focus ring come with it. `p-1` / `rounded` become pad + radius.
  const headerIconVars = {
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-bg-hover': 'rgba(255,255,255,0.05)',
    '--pa-btn-pad': '4px',
    '--pa-btn-radius': `${radius.xs}px`,
  } as CSSProperties;

  return (
    <div className="flex h-full" style={{ backgroundColor: colors.bg }}>
      {/* Left: Skills list */}
      <div
        className={`flex flex-col ${selectedSkill ? 'w-1/2' : 'w-full'}`}
        style={selectedSkill ? { borderRight: `1px solid ${colors.border}` } : undefined}
      >
        {/* Header */}
        <div
          className="flex items-center justify-between px-4 py-2.5"
          style={{ borderBottom: `1px solid ${colors.border}` }}
        >
          <span
            className="text-[11px] uppercase tracking-wider"
            style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
          >
            Skills Library
          </span>
          <div className="flex items-center gap-2">
            <span
              className="rounded px-1.5 py-0.5 text-[10px]"
              style={{ fontFamily: font.mono, backgroundColor: colors.surface, color: colors.textMuted }}
            >
              {filtered.length}
            </span>
            <Tooltip content={viewMode === 'list' ? 'Grid view' : 'List view'}>
              <Button
                colors={colors}
                variant="bare"
                type="button"
                onClick={() => setViewMode(viewMode === 'list' ? 'grid' : 'list')}
                aria-label={viewMode === 'list' ? 'Grid view' : 'List view'}
                style={headerIconVars}
              >
                {viewMode === 'list' ? <FiGrid size={13} /> : <FiList size={13} />}
              </Button>
            </Tooltip>
            {onClose && (
              <Tooltip content="Close (Esc)">
                <Button
                  colors={colors}
                  variant="bare"
                  type="button"
                  onClick={onClose}
                  aria-label="Close skills library"
                  style={headerIconVars}
                >
                  <FiX size={14} />
                </Button>
              </Tooltip>
            )}
          </div>
        </div>

        {/* Search */}
        <div className="px-4 py-2" style={{ borderBottom: `1px solid ${colors.border}` }}>
          <div className="relative">
            <FiSearch size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2" style={{ color: colors.textMuted, opacity: 0.5 }} />
            <input
              type="text"
              value={search}
              onChange={e => setSearch(e.target.value)}
              placeholder="Search skills..."
              className="skills-search-input w-full rounded-md pl-7 pr-3 py-1.5 text-[11px] focus:outline-none transition"
              style={{ fontFamily: font.mono, border: `1px solid ${colors.border}`, backgroundColor: `${colors.surface}80`, color: colors.text }}
              onFocus={e => { e.currentTarget.style.borderColor = `${colors.cyan}80`; }}
              onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
            />
            <style>{`.skills-search-input::placeholder { color: ${colors.textMuted}; opacity: 0.6; }`}</style>
          </div>
        </div>

        {/* Skills list */}
        <div className="flex-1 overflow-y-auto p-4">
          {skillsLoading ? (
            <div className="flex items-center justify-center h-32" style={{ color: colors.textMuted }}>
              <FiLoader size={16} className="animate-spin mr-2" />
              <span className="text-xs" style={{ fontFamily: font.mono }}>Loading skills...</span>
            </div>
          ) : skillsError ? (
            // A failed load is not an empty library. The copy below is a good
            // invitation and a false statement when the fetch never landed.
            <div
              data-testid="skills-load-error"
              className="flex flex-col items-center justify-center h-full text-xs text-center gap-2"
              style={{ fontFamily: font.mono, color: colors.textMuted }}
            >
              <div style={{ color: colors.danger, fontWeight: 600 }}>Couldn't load your skills</div>
              <div className="text-[10px]">{skillsError}</div>
              <Button
                colors={colors}
                variant="ghostOn"
                type="button"
                onClick={async () => {
                  await loadSkills();
                  // `loadSkills` swallows its own failure into `skillsError`, so
                  // without this the retry would tick "done" over an error that
                  // is still on screen. `false` is the primitive's "it failed".
                  return !useCommandCenter.getState().skillsError;
                }}
                style={{
                  '--pa-btn-bg': colors.cyanSoft,
                  '--pa-btn-fg': colors.cyan,
                  '--pa-btn-border': colors.borderHi,
                  '--pa-btn-bg-hover': `${colors.cyan}26`,
                  '--pa-btn-border-hover': colors.cyan,
                  '--pa-btn-bg-active': colors.cyanSoft,
                  '--pa-btn-pad': '4px 12px',
                  '--pa-btn-radius': `${radius.sm}px`,
                  '--pa-btn-weight': 600,
                  marginTop: space.xs,
                  fontFamily: font.body,
                  fontSize: 10,
                  lineHeight: '15px',
                } as CSSProperties}
              >
                Try again
              </Button>
            </div>
          ) : filtered.length === 0 ? (
            <div
              data-testid="skills-empty"
              className="flex flex-col items-center justify-center h-full text-xs text-center gap-2"
              style={{ fontFamily: font.mono, color: colors.textMuted }}
            >
              <FiZap size={24} className="opacity-30" style={{ color: `${colors.cyan}66` }} />
              {search.trim() ? (
                <div>No skills match "{search}"</div>
              ) : (
                <>
                  <div>No skills saved yet.</div>
                  <div className="text-[10px]">Skills are created when your agent detects repeated patterns.</div>
                </>
              )}
            </div>
          ) : viewMode === 'grid' ? (
            <div className="grid grid-cols-2 gap-3">
              {filtered.map(skill => (
                <SkillCard
                  key={skill.id}
                  skill={skill}
                  isSelected={skill.id === selectedSkillId}
                  onSelect={setSelectedSkillId}
                />
              ))}
            </div>
          ) : (
            <div className="space-y-2">
              {filtered.map(skill => (
                <SkillCard
                  key={skill.id}
                  skill={skill}
                  isSelected={skill.id === selectedSkillId}
                  onSelect={setSelectedSkillId}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Right: Detail panel */}
      {selectedSkill && (
        <div className="w-1/2">
          <SkillDetailPanel skill={selectedSkill} />
        </div>
      )}
    </div>
  );
}
