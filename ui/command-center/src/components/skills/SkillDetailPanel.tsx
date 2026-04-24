import { useState } from 'react';
import { FiTrash2, FiEdit2, FiX, FiChevronDown, FiChevronRight } from 'react-icons/fi';
import { useCommandCenter, type SkillState } from '../../lib/store';
import { SkillEditor } from './SkillEditor';
import { SkillExecutionHistory } from './SkillExecutionHistory';

interface SkillDetailPanelProps {
  skill: SkillState;
}

export function SkillDetailPanel({ skill }: SkillDetailPanelProps) {
  const deleteSkill = useCommandCenter(s => s.deleteSkill);
  const setSelectedSkillId = useCommandCenter(s => s.setSelectedSkillId);
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [showDefinition, setShowDefinition] = useState(false);

  const handleDelete = async () => {
    await deleteSkill(skill.id);
    setSelectedSkillId(null);
  };

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-dark-border px-4 py-2.5">
        <div className="flex items-center gap-2 min-w-0">
          <span className={`w-2 h-2 rounded-full shrink-0 ${
            skill.status === 'active' ? 'bg-emerald-400' : 'bg-slate-500'
          }`} />
          <span className="text-sm font-medium text-dark-text truncate">{skill.name}</span>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <button
            onClick={() => setEditing(!editing)}
            className="rounded p-1.5 text-dark-muted hover:text-accent hover:bg-accent/10 transition"
            title="Edit skill"
          >
            <FiEdit2 size={13} />
          </button>
          <button
            onClick={() => setConfirmDelete(true)}
            className="rounded p-1.5 text-dark-muted hover:text-red-400 hover:bg-red-400/10 transition"
            title="Delete skill"
          >
            <FiTrash2 size={13} />
          </button>
          <button
            onClick={() => setSelectedSkillId(null)}
            className="rounded p-1.5 text-dark-muted hover:text-dark-text hover:bg-white/5 transition"
            title="Close"
          >
            <FiX size={14} />
          </button>
        </div>
      </div>

      {/* Delete confirmation */}
      {confirmDelete && (
        <div className="mx-4 mt-3 rounded-lg border border-red-400/30 bg-red-400/5 p-3">
          <p className="text-xs text-red-300 mb-2 font-mono">Delete this skill permanently?</p>
          <div className="flex gap-2">
            <button
              onClick={handleDelete}
              className="rounded-md bg-red-500/20 px-3 py-1 text-[11px] font-mono text-red-400 hover:bg-red-500/30 transition"
            >
              Delete
            </button>
            <button
              onClick={() => setConfirmDelete(false)}
              className="rounded-md px-3 py-1 text-[11px] font-mono text-dark-muted hover:text-dark-text hover:bg-white/5 transition"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {editing ? (
          <SkillEditor skill={skill} onClose={() => setEditing(false)} />
        ) : (
          <>
            {/* Description */}
            {skill.description && (
              <div>
                <label className="block text-[10px] font-mono uppercase text-dark-muted mb-1">Description</label>
                <p className="text-xs text-dark-text/80">{skill.description}</p>
              </div>
            )}

            {/* Metadata grid */}
            <div className="grid grid-cols-2 gap-3">
              <MetaField label="Trigger Type" value={skill.trigger_type || 'manual'} />
              <MetaField label="Status" value={skill.status || 'active'} />
              <MetaField label="Usage Count" value={String(skill.usage_count ?? 0)} />
              <MetaField label="Version" value={skill.version || '1'} />
              <MetaField label="Created" value={skill.created_at ? new Date(skill.created_at).toLocaleString() : '--'} />
              <MetaField label="Last Run" value={skill.last_run ? new Date(skill.last_run).toLocaleString() : 'Never'} />
            </div>

            {/* Steps */}
            {skill.steps && skill.steps.length > 0 && (
              <div>
                <label className="block text-[10px] font-mono uppercase text-dark-muted mb-2">Steps</label>
                <div className="space-y-1.5">
                  {skill.steps.map((step, i) => (
                    <div key={i} className="flex items-start gap-2 rounded bg-dark-surface/50 p-2">
                      <span className="text-[10px] font-mono text-dark-muted shrink-0 mt-0.5">{i + 1}.</span>
                      <div className="min-w-0">
                        <span className="text-xs text-dark-text">{step.action}</span>
                        {step.tool && (
                          <span className="ml-2 text-[10px] font-mono text-accent/70">{step.tool}</span>
                        )}
                        {step.description && (
                          <p className="text-[10px] text-dark-muted mt-0.5">{step.description}</p>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Definition JSON (collapsible) */}
            <div>
              <button
                onClick={() => setShowDefinition(!showDefinition)}
                className="flex items-center gap-1 text-[10px] font-mono uppercase text-dark-muted hover:text-dark-text transition"
              >
                {showDefinition ? <FiChevronDown size={11} /> : <FiChevronRight size={11} />}
                Definition JSON
              </button>
              {showDefinition && (
                <pre className="mt-2 rounded bg-dark-surface/50 p-3 text-[10px] font-mono text-dark-muted overflow-x-auto max-h-64 overflow-y-auto">
                  {JSON.stringify(skill, null, 2)}
                </pre>
              )}
            </div>

            {/* Execution history */}
            <SkillExecutionHistory skillId={skill.id} />
          </>
        )}
      </div>
    </div>
  );
}

function MetaField({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <label className="block text-[10px] font-mono uppercase text-dark-muted mb-0.5">{label}</label>
      <span className="text-xs text-dark-text/80 font-mono">{value}</span>
    </div>
  );
}
