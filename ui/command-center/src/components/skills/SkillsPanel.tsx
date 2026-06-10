import { useEffect, useState, useMemo } from 'react';
import { FiLoader, FiZap, FiSearch, FiGrid, FiList } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';
import { SkillCard } from './SkillCard';
import { SkillDetailPanel } from './SkillDetailPanel';

export function SkillsPanel() {
  const { colors } = useTheme();
  const skills = useCommandCenter(s => s.skills);
  const skillsLoading = useCommandCenter(s => s.skillsLoading);
  const loadSkills = useCommandCenter(s => s.loadSkills);
  const selectedSkillId = useCommandCenter(s => s.selectedSkillId);
  const setSelectedSkillId = useCommandCenter(s => s.setSelectedSkillId);

  const [search, setSearch] = useState('');
  const [viewMode, setViewMode] = useState<'list' | 'grid'>('list');

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

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

  return (
    <div className="flex h-full" style={{ backgroundColor: colors.bg }}>
      {/* Left: Skills list */}
      <div className={`flex flex-col ${selectedSkill ? 'w-1/2 border-r border-dark-border' : 'w-full'}`}>
        {/* Header */}
        <div className="flex items-center justify-between border-b border-dark-border px-4 py-2.5">
          <span className="text-[11px] font-mono uppercase tracking-wider text-dark-muted">
            Skills Library
          </span>
          <div className="flex items-center gap-2">
            <span className="rounded bg-dark-surface px-1.5 py-0.5 text-[10px] font-mono text-dark-muted">
              {filtered.length}
            </span>
            <button
              onClick={() => setViewMode(viewMode === 'list' ? 'grid' : 'list')}
              className="rounded p-1 text-dark-muted hover:text-dark-text hover:bg-white/5 transition"
              title={viewMode === 'list' ? 'Grid view' : 'List view'}
            >
              {viewMode === 'list' ? <FiGrid size={13} /> : <FiList size={13} />}
            </button>
          </div>
        </div>

        {/* Search */}
        <div className="px-4 py-2 border-b border-dark-border">
          <div className="relative">
            <FiSearch size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-dark-muted/50" />
            <input
              type="text"
              value={search}
              onChange={e => setSearch(e.target.value)}
              placeholder="Search skills..."
              className="w-full rounded-md border border-dark-border bg-dark-surface/50 pl-7 pr-3 py-1.5 text-[11px] font-mono text-dark-text placeholder-dark-muted/60 focus:border-accent/50 focus:outline-none transition"
            />
          </div>
        </div>

        {/* Skills list */}
        <div className="flex-1 overflow-y-auto p-4">
          {skillsLoading ? (
            <div className="flex items-center justify-center h-32 text-dark-muted">
              <FiLoader size={16} className="animate-spin mr-2" />
              <span className="text-xs font-mono">Loading skills...</span>
            </div>
          ) : filtered.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full text-dark-muted text-xs text-center gap-2 font-mono">
              <FiZap size={24} className="opacity-30 text-accent/40" />
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
