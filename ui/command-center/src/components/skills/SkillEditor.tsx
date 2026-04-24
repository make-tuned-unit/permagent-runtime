import { useState } from 'react';
import { FiCheck, FiX } from 'react-icons/fi';
import { useCommandCenter, type SkillState } from '../../lib/store';

interface SkillEditorProps {
  skill: SkillState;
  onClose: () => void;
}

export function SkillEditor({ skill, onClose }: SkillEditorProps) {
  const updateSkill = useCommandCenter(s => s.updateSkill);
  const [name, setName] = useState(skill.name);
  const [description, setDescription] = useState(skill.description || '');
  const [saving, setSaving] = useState(false);

  const hasChanges = name !== skill.name || description !== (skill.description || '');

  const handleSave = async () => {
    if (!hasChanges || !name.trim()) return;
    setSaving(true);
    await updateSkill(skill.id, { name: name.trim(), description: description.trim() });
    setSaving(false);
    onClose();
  };

  return (
    <div className="space-y-3">
      <div>
        <label className="block text-[10px] font-mono uppercase text-dark-muted mb-1">Name</label>
        <input
          type="text"
          value={name}
          onChange={e => setName(e.target.value)}
          className="w-full rounded-md border border-dark-border bg-dark-surface/50 px-3 py-1.5 text-xs text-dark-text placeholder-dark-muted/50 focus:border-accent/50 focus:outline-none transition"
          placeholder="Skill name"
        />
      </div>
      <div>
        <label className="block text-[10px] font-mono uppercase text-dark-muted mb-1">Description</label>
        <textarea
          value={description}
          onChange={e => setDescription(e.target.value)}
          rows={3}
          className="w-full rounded-md border border-dark-border bg-dark-surface/50 px-3 py-1.5 text-xs text-dark-text placeholder-dark-muted/50 focus:border-accent/50 focus:outline-none transition resize-none"
          placeholder="What does this skill do?"
        />
      </div>
      <div className="flex items-center gap-2">
        <button
          onClick={handleSave}
          disabled={!hasChanges || !name.trim() || saving}
          className="flex items-center gap-1 rounded-md bg-accent/20 px-3 py-1 text-[11px] font-mono text-accent hover:bg-accent/30 transition disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <FiCheck size={12} />
          {saving ? 'Saving...' : 'Save'}
        </button>
        <button
          onClick={onClose}
          className="flex items-center gap-1 rounded-md px-3 py-1 text-[11px] font-mono text-dark-muted hover:text-dark-text hover:bg-white/5 transition"
        >
          <FiX size={12} />
          Cancel
        </button>
      </div>
    </div>
  );
}
