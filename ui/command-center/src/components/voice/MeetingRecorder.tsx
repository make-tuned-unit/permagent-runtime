/**
 * MeetingRecorder — the sidebar "Record" button + confirm-first project picker
 * + always-visible recording indicator (call-notes MVP 1A).
 *
 * Flow (ratified rulings baked into the UI):
 *  1. Toolbar button (NOT push-to-talk — spacebar PTT dies when the embedded
 *     webview holds focus; a click-to-toggle toolbar button does not).
 *  2. Confirm-first: clicking Record opens a project picker; nothing records
 *     until the user picks a project and explicitly starts. The modal states
 *     plainly what will happen: own voice only (this machine's mic — never the
 *     other side of a call), transcribed LOCALLY by the on-device Whisper
 *     model, saved as a note on the chosen project when stopped.
 *  3. While recording, a fixed indicator pill stays visible (pulsing dot +
 *     elapsed time + project) with Stop & save and a two-step Discard.
 *  4. Stop → useMeetingDictation flushes, transcribes the tail, and saves the
 *     note via the existing notes path; success lands as a toast. A failed
 *     save keeps the transcript and offers Retry — words are never dropped.
 *
 * Lives inside the Sidebar (which never unmounts in the main window), so a
 * recording survives workspace/overlay switches. Modal + indicator render
 * through portals so the collapsed sidebar never clips them.
 */

import { useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { apiFetch } from '../../lib/api';
import { toast } from '../../lib/notifications';
import { font, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useMeetingDictation, formatElapsed } from '../../hooks/useMeetingDictation';
import type { Project } from '../projects/types';

/** Mic glyph (same stroke style as the sidebar's other icons). */
const MIC_ICON = 'M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3zM19 10v2a7 7 0 0 1-14 0v-2M12 19v4M8 23h8';

export function MeetingRecorder({ open }: { open: boolean }) {
  const { colors, gradient } = useTheme();
  const {
    state, error, elapsedSeconds, failedChunks, target, hasUnsavedTranscript,
    start, stop, retrySave, discard,
    systemAudio, setSystemAudio, systemAudioError, systemAudioAvailable,
  } = useMeetingDictation();
  // Whether this build carries the capture helper at all. Checked rather than
  // assumed so the toggle is never offered where it cannot work.
  const [canCaptureSystem, setCanCaptureSystem] = useState(false);
  useEffect(() => { void systemAudioAvailable().then(setCanCaptureSystem); }, [systemAudioAvailable]);

  const [pickerOpen, setPickerOpen] = useState(false);
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);

  const busy = state === 'recording' || state === 'finishing';

  const loadProjects = useCallback(() => {
    setLoadError(false);
    apiFetch<Project[]>('/api/projects')
      .then(list => setProjects(list.filter(p => p.status !== 'archived')))
      .catch(() => { setProjects(null); setLoadError(true); });
  }, []);

  useEffect(() => { if (pickerOpen) loadProjects(); }, [pickerOpen, loadProjects]);
  useEffect(() => { if (state !== 'recording') setConfirmDiscard(false); }, [state]);

  const onButton = () => {
    if (state === 'recording') { void handleStop(); return; }
    if (state === 'finishing') return; // saving — let it finish
    setPickerOpen(true);
  };

  const handleStart = async () => {
    const project = projects?.find(p => p.id === selectedId);
    if (!project) return;
    setPickerOpen(false);
    await start({ projectId: project.id, projectName: project.name });
  };

  const handleStop = async () => {
    const name = target?.projectName;
    const ok = await stop();
    if (ok && name) toast('Meeting note saved', `The transcript was saved as a note on "${name}".`);
  };

  const handleRetry = async () => {
    const name = target?.projectName;
    const ok = await retrySave();
    if (ok && name) toast('Meeting note saved', `The transcript was saved as a note on "${name}".`);
  };

  const active = busy || state === 'error';

  return (
    <>
      {/* Sidebar row (mirrors SidebarRow styling; red while recording). */}
      <button
        onClick={onButton}
        title={
          state === 'recording' ? 'Stop recording and save the note'
            : state === 'finishing' ? 'Transcribing & saving…'
            : 'Record a meeting to a project note'
        }
        aria-label="Record meeting"
        style={{
          width: open ? 'calc(100% - 16px)' : 40,
          height: 40, borderRadius: 10,
          display: 'flex', alignItems: 'center', gap: 12,
          padding: open ? '0 12px' : 0,
          justifyContent: open ? 'flex-start' : 'center',
          margin: open ? '0 8px' : '0 auto',
          background: state === 'recording' ? colors.danger + '24' : active ? colors.cyanSoft : 'transparent',
          border: state === 'recording' ? `1px solid ${colors.danger}` : active ? `1px solid ${colors.borderHi}` : '1px solid transparent',
          color: state === 'recording' ? colors.danger : active ? colors.cyan : colors.textMuted,
          cursor: state === 'finishing' ? 'default' : 'pointer',
          transition: `all 200ms ${ease.out}`,
          fontFamily: font.body, fontSize: 13, fontWeight: active ? 600 : 500,
          textAlign: 'left',
        }}
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" style={{ flexShrink: 0 }}
          stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round">
          <path d={MIC_ICON} />
        </svg>
        {open && (
          <span style={{ whiteSpace: 'nowrap' }}>
            {state === 'recording' ? `Recording ${formatElapsed(elapsedSeconds)}`
              : state === 'finishing' ? 'Saving…'
              : 'Record'}
          </span>
        )}
      </button>

      {/* Confirm-first project picker. */}
      {pickerOpen && createPortal(
        <div
          onMouseDown={e => { if (e.target === e.currentTarget) setPickerOpen(false); }}
          style={{
            position: 'fixed', inset: 0, zIndex: 1000,
            background: 'rgba(0,0,0,0.45)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}
        >
          <div style={{
            width: 380, maxWidth: 'calc(100vw - 48px)', maxHeight: '70vh',
            display: 'flex', flexDirection: 'column',
            background: gradient.dropdown, backdropFilter: 'blur(16px)',
            border: `1px solid ${colors.borderHi}`, borderRadius: 12,
            boxShadow: '0 12px 40px rgba(0,0,0,0.6)',
            padding: 16, fontFamily: font.body,
          }}>
            <div style={{ fontSize: 14, fontWeight: 600, color: colors.text, marginBottom: 4 }}>
              Record a meeting
            </div>
            <div style={{ fontSize: 11, color: colors.textMuted, lineHeight: 1.5, marginBottom: 10 }}>
              {systemAudio
                ? "Records BOTH sides — your microphone and this Mac's audio output, which is the other participants. Transcribed locally on this device; nothing is sent to a cloud service. Saved as a note on the project you pick."
                : "Records your own voice from this machine's microphone (not the other side of a call), transcribes it locally on this device — nothing is sent to a cloud service — and saves the transcript as a note on the project you pick when you stop."}
            </div>

            {/* Far-side capture. Off by default and stated plainly: recording
                the other participants is a materially different act from
                recording yourself, and it should never be a silent default.
                macOS asks for Screen Recording the first time — that grant is
                what makes system audio reachable at all. */}
            {canCaptureSystem && (
              <label style={{
                display: 'flex', alignItems: 'flex-start', gap: 8, marginBottom: 10,
                fontSize: 11, color: colors.textMuted, lineHeight: 1.5, cursor: 'pointer',
              }}>
                <input
                  type="checkbox"
                  checked={systemAudio}
                  onChange={e => setSystemAudio(e.target.checked)}
                  style={{ marginTop: 2, accentColor: colors.cyan }}
                />
                <span>
                  <span style={{ color: colors.text, fontWeight: 600 }}>Also record the other participants</span>
                  {' '}— captures this Mac's audio output so the transcript has both
                  sides of the call. macOS will ask for Screen Recording permission
                  the first time. Check that everyone on the call is happy to be
                  recorded.
                </span>
              </label>
            )}

            {systemAudioError && (
              <div style={{
                fontSize: 11, color: colors.danger, marginBottom: 8, lineHeight: 1.5,
              }}>{systemAudioError}</div>
            )}

            {loadError && (
              <div style={{ fontSize: 11, color: colors.danger, marginBottom: 8, display: 'flex', gap: 8, alignItems: 'center' }}>
                Couldn't load projects.
                <button onClick={loadProjects} style={{
                  fontSize: 11, color: colors.cyan, background: 'none', border: 'none',
                  cursor: 'pointer', fontFamily: font.body, padding: 0, fontWeight: 600,
                }}>Retry</button>
              </div>
            )}
            {!loadError && projects === null && (
              <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 8 }}>Loading projects…</div>
            )}
            {projects !== null && (
              <div style={{ overflow: 'auto', flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', gap: 4, marginBottom: 12 }}>
                {projects.length === 0 && (
                  <div style={{ fontSize: 11, color: colors.textDim }}>No projects available.</div>
                )}
                {projects.map(p => (
                  <button
                    key={p.id}
                    onClick={() => setSelectedId(p.id)}
                    style={{
                      textAlign: 'left', padding: '8px 10px', borderRadius: 8,
                      background: selectedId === p.id ? colors.cyanSoft : 'transparent',
                      border: `1px solid ${selectedId === p.id ? colors.borderHi : colors.border}`,
                      color: selectedId === p.id ? colors.cyan : colors.text,
                      fontSize: 12, fontFamily: font.body, cursor: 'pointer', fontWeight: selectedId === p.id ? 600 : 400,
                    }}
                  >
                    {p.name}
                  </button>
                ))}
              </div>
            )}

            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button onClick={() => setPickerOpen(false)} style={{
                fontSize: 12, padding: '7px 14px', borderRadius: 7, cursor: 'pointer',
                background: 'transparent', border: `1px solid ${colors.border}`,
                color: colors.textMuted, fontFamily: font.body,
              }}>
                Cancel
              </button>
              <button
                onClick={handleStart}
                disabled={!selectedId}
                style={{
                  fontSize: 12, fontWeight: 600, padding: '7px 14px', borderRadius: 7,
                  cursor: selectedId ? 'pointer' : 'default', opacity: selectedId ? 1 : 0.5,
                  background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
                  color: colors.cyan, fontFamily: font.body,
                }}
              >
                Start recording
              </button>
            </div>
          </div>
        </div>,
        document.body,
      )}

      {/* Always-visible recording / saving / error indicator. */}
      {(busy || state === 'error') && createPortal(
        <div style={{
          position: 'fixed', right: 16, bottom: 16, zIndex: 999,
          maxWidth: 340,
          background: gradient.dropdown, backdropFilter: 'blur(16px)',
          border: `1px solid ${state === 'recording' ? colors.danger : colors.borderHi}`,
          borderRadius: 12, boxShadow: '0 12px 40px rgba(0,0,0,0.6)',
          padding: '10px 14px', fontFamily: font.body,
          display: 'flex', flexDirection: 'column', gap: 6,
        }}>
          <style>{'@keyframes pa-rec-pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.25; } }'}</style>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            {state === 'recording' && (
              <span style={{
                width: 9, height: 9, borderRadius: '50%', background: colors.danger,
                animation: 'pa-rec-pulse 1.4s ease-in-out infinite', flexShrink: 0,
              }} />
            )}
            <span style={{ fontSize: 12, fontWeight: 600, color: state === 'error' ? colors.danger : colors.text }}>
              {state === 'recording' && `Recording ${formatElapsed(elapsedSeconds)}`}
              {state === 'finishing' && 'Transcribing & saving…'}
              {state === 'error' && 'Meeting dictation'}
            </span>
            {target && (
              <span style={{ fontSize: 11, color: colors.textDim, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                → {target.projectName}
              </span>
            )}
          </div>

          {state === 'recording' && (
            <div style={{ fontSize: 10, color: colors.textDim }}>
              Own voice only · transcribed locally on this device
              {failedChunks > 0 && ` · ${failedChunks} segment${failedChunks > 1 ? 's' : ''} failed to transcribe`}
            </div>
          )}

          {error && (
            <div style={{ fontSize: 11, color: colors.danger, lineHeight: 1.4 }}>{error}</div>
          )}

          <div style={{ display: 'flex', gap: 8 }}>
            {state === 'recording' && (
              <>
                <button onClick={handleStop} style={{
                  fontSize: 11, fontWeight: 600, padding: '5px 12px', borderRadius: 7, cursor: 'pointer',
                  background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
                  color: colors.cyan, fontFamily: font.body,
                }}>
                  Stop & save
                </button>
                <button onClick={() => (confirmDiscard ? discard() : setConfirmDiscard(true))} style={{
                  fontSize: 11, padding: '5px 12px', borderRadius: 7, cursor: 'pointer',
                  background: confirmDiscard ? colors.danger + '24' : 'transparent',
                  border: `1px solid ${confirmDiscard ? colors.danger : colors.border}`,
                  color: confirmDiscard ? colors.danger : colors.textMuted, fontFamily: font.body,
                }}>
                  {confirmDiscard ? 'Discard — sure?' : 'Discard'}
                </button>
              </>
            )}
            {state === 'error' && (
              <>
                {hasUnsavedTranscript && (
                  <button onClick={handleRetry} style={{
                    fontSize: 11, fontWeight: 600, padding: '5px 12px', borderRadius: 7, cursor: 'pointer',
                    background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
                    color: colors.cyan, fontFamily: font.body,
                  }}>
                    Retry save
                  </button>
                )}
                <button onClick={discard} style={{
                  fontSize: 11, padding: '5px 12px', borderRadius: 7, cursor: 'pointer',
                  background: 'transparent', border: `1px solid ${colors.border}`,
                  color: colors.textMuted, fontFamily: font.body,
                }}>
                  Dismiss
                </button>
              </>
            )}
          </div>
        </div>,
        document.body,
      )}
    </>
  );
}
