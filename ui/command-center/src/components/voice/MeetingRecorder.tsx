/**
 * MeetingRecorder — the sidebar "Record" button + confirm-first project picker
 * + always-visible recording panel (call-notes MVP 1A).
 *
 * Flow (ratified rulings baked into the UI):
 *  1. Toolbar button (NOT push-to-talk — spacebar PTT dies when the embedded
 *     webview holds focus; a click-to-toggle toolbar button does not).
 *  2. Confirm-first: clicking Record opens a project picker; nothing records
 *     until the user picks a project and explicitly starts. The modal states
 *     plainly what will happen: which sides are captured, that transcription is
 *     LOCAL, and that the result lands as a note on the chosen project.
 *  3. While recording, a fixed panel stays visible — elapsed time, live proof
 *     that each side is actually being heard, the notepad, and Stop & save with
 *     a two-step Discard.
 *  4. Stop → useMeetingDictation flushes, transcribes the tail, and saves the
 *     note via the existing notes path; success lands as a toast. A failed
 *     save keeps the transcript and offers Retry — words are never dropped.
 *
 * The notepad is the surface the user actually looks at during a call, so it
 * gets room to write in: the panel expands to a real editor and remembers that
 * choice for the session. Granola's insight is that what the user types STEERS
 * the summary — a three-line box in the corner quietly says "don't bother".
 *
 * Lives inside the Sidebar (which never unmounts in the main window), so a
 * recording survives workspace/overlay switches. Modal + panel render through
 * portals so the collapsed sidebar never clips them.
 */

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { useCommandCenter } from '../../lib/store';
import { apiFetch } from '../../lib/api';
import { toast } from '../../lib/notifications';
import { ease, font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useMeetingDictation, formatElapsed } from '../../hooks/useMeetingDictation';
import { Button } from '../common/Button';
import type { Project } from '../projects/types';

/** Mic glyph (same stroke style as the sidebar's other icons). */
const MIC_ICON = 'M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3zM19 10v2a7 7 0 0 1-14 0v-2M12 19v4M8 23h8';
/** Chevron for the panel's expand/collapse affordance. */
const CHEVRON = 'M6 9l6 6 6-6';

/** Above this many projects the picker gets a filter field — scrolling a long
 *  list to start a recording is friction at exactly the wrong moment. */
const FILTER_THRESHOLD = 6;

export function MeetingRecorder({ open }: { open: boolean }) {
  const { colors, gradient } = useTheme();
  const {
    state, error, elapsedSeconds, failedChunks, target, hasUnsavedTranscript,
    start, stop, retrySave, discard,
    systemAudio, setSystemAudio, systemAudioError, systemAudioAvailable,
    farChunksHeard, nearChunksHeard,
    recoveredDrafts, recoverDraft, dismissDraft,
    userNotes, setUserNotes,
  } = useMeetingDictation();
  const [recovering, setRecovering] = useState<string | null>(null);
  // Whether this build carries the capture helper at all. Checked rather than
  // assumed so the toggle is never offered where it cannot work.
  const [canCaptureSystem, setCanCaptureSystem] = useState(false);
  useEffect(() => { void systemAudioAvailable().then(setCanCaptureSystem); }, [systemAudioAvailable]);

  const [pickerOpen, setPickerOpen] = useState(false);
  // The native browser webview composites above ALL DOM (the corner-cede
  // trap) — without this, the picker modal opens BEHIND an open web call.
  const pushBrowserOverlay = useCommandCenter(s => s.pushBrowserOverlay);
  const popBrowserOverlay = useCommandCenter(s => s.popBrowserOverlay);
  useEffect(() => {
    if (!pickerOpen) return;
    pushBrowserOverlay();
    return () => popBrowserOverlay();
  }, [pickerOpen, pushBrowserOverlay, popBrowserOverlay]);
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  /** The notepad given room to write in. Sticky for the session. */
  const [expanded, setExpanded] = useState(false);
  const notepadRef = useRef<HTMLTextAreaElement | null>(null);
  const filterRef = useRef<HTMLInputElement | null>(null);

  const busy = state === 'recording' || state === 'finishing';

  // Dock the live panel into the chat sidebar when it's open: the dock is a
  // flex SIBLING of <main>, so it sits beside the native browser webview
  // rather than under it — the floating bottom-right card was invisible
  // during a web call (the corner-cede trap) and collided with the chat pill.
  const chatDockOpen = useCommandCenter(s => s.chatDockOpen);
  const [dockSlot, setDockSlot] = useState<HTMLElement | null>(null);
  useEffect(() => {
    const wantDock = chatDockOpen && (busy || state === 'error');
    if (!wantDock) { setDockSlot(null); return; }
    // The dock mounts in the same commit as chatDockOpen flips — look for the
    // slot now and once more on the next tick.
    const find = () => setDockSlot(document.getElementById('meeting-dock-slot'));
    find();
    const t = setTimeout(find, 50);
    return () => clearTimeout(t);
  }, [chatDockOpen, busy, state]);
  const docked = dockSlot !== null;

  const loadProjects = useCallback(() => {
    setLoadError(false);
    apiFetch<Project[]>('/api/projects')
      .then(list => setProjects(list.filter(p => p.status !== 'archived')))
      .catch(() => { setProjects(null); setLoadError(true); });
  }, []);

  useEffect(() => { if (pickerOpen) loadProjects(); }, [pickerOpen, loadProjects]);
  useEffect(() => { if (state !== 'recording') setConfirmDiscard(false); }, [state]);

  const visibleProjects = useMemo(() => {
    if (!projects) return [];
    const needle = filter.trim().toLowerCase();
    return needle ? projects.filter(p => p.name.toLowerCase().includes(needle)) : projects;
  }, [projects, filter]);

  const handleStart = useCallback(async () => {
    const project = visibleProjects.find(p => p.id === selectedId);
    if (!project) return;
    setPickerOpen(false);
    setFilter('');
    await start({ projectId: project.id, projectName: project.name });
    // The notepad is the point of the panel — put the cursor in it.
    requestAnimationFrame(() => notepadRef.current?.focus());
  }, [visibleProjects, selectedId, start]);

  // Escape closes the picker and Enter starts with the current selection: a
  // modal you can only leave with the mouse is the one that feels stuck.
  useEffect(() => {
    if (!pickerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.stopPropagation(); setPickerOpen(false); setFilter(''); }
      if (e.key === 'Enter' && selectedId) { e.preventDefault(); void handleStart(); }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [pickerOpen, selectedId, handleStart]);

  // A single match after filtering is unambiguous — preselect it so Enter works.
  useEffect(() => {
    if (visibleProjects.length === 1) setSelectedId(visibleProjects[0].id);
    else if (selectedId && !visibleProjects.some(p => p.id === selectedId)) setSelectedId(null);
  }, [visibleProjects, selectedId]);

  useEffect(() => {
    if (pickerOpen && projects && projects.length > FILTER_THRESHOLD) filterRef.current?.focus();
  }, [pickerOpen, projects]);

  const onButton = () => {
    if (state === 'recording') {
      // While recording, the rail button SURFACES the panel (docked in the
      // chat sidebar) instead of stop-saving — a collapsed-rail misclick must
      // never end a live meeting. Stop & save lives on the panel.
      useCommandCenter.getState().openChatDock();
      return;
    }
    if (state === 'finishing') return; // saving — let it finish
    setPickerOpen(true);
  };

  /** Deep-link toast: clicking it lands on the note, expanded, in Projects. */
  const savedToast = (body: string, saved: { projectId: string; noteId: string }) => {
    toast('Meeting note saved', body, () =>
      useCommandCenter.getState().focusProjectNote(saved.projectId, saved.noteId));
  };

  // Both of these resolve to whether a note actually landed: `stop`/`retrySave`
  // report failure by returning null and setting `error`, so a button that
  // ticked on a null would be claiming a save that did not happen.
  const handleStop = async () => {
    const name = target?.projectName;
    const saved = await stop();
    if (saved && name) {
      savedToast(`Writing up the notes for "${name}" now — click to open the note.`, saved);
    }
    return saved !== null;
  };

  const handleRetry = async () => {
    const name = target?.projectName;
    const saved = await retrySave();
    if (saved && name) savedToast(`The transcript was saved as a note on "${name}" — click to open it.`, saved);
    return saved !== null;
  };

  const active = busy || state === 'error';
  const panelWidth = expanded ? 520 : 360;

  /** Shared button look — the panel had five near-identical inline copies, and
   *  an inline style cannot express hover or press. Same three faces, now fed
   *  to `.pa-btn` as custom properties so the states come with them. */
  const btnVars = (kind: 'primary' | 'quiet' | 'danger'): CSSProperties => {
    const fill = kind === 'primary' ? colors.cyanSoft : kind === 'danger' ? colors.danger + '24' : 'transparent';
    const line = kind === 'primary' ? colors.borderHi : kind === 'danger' ? colors.danger : colors.border;
    const ink = kind === 'primary' ? colors.cyan : kind === 'danger' ? colors.danger : colors.textMuted;
    return {
      '--pa-btn-bg': fill,
      '--pa-btn-fg': ink,
      '--pa-btn-border': line,
      '--pa-btn-bg-hover': kind === 'quiet' ? colors.surfaceHi : kind === 'danger' ? colors.danger + '38' : fill,
      '--pa-btn-border-hover': kind === 'quiet' ? colors.borderHi : kind === 'primary' ? colors.cyan : line,
      '--pa-btn-fg-hover': kind === 'quiet' ? colors.text : ink,
      '--pa-btn-bg-active': fill,
      '--pa-btn-pad': '5px 12px',
      '--pa-btn-radius': '7px',
      '--pa-btn-weight': kind === 'primary' ? 600 : 400,
      fontFamily: font.body,
      fontSize: 11,
    } as CSSProperties;
  };

  /** The floating card both the recovery prompt and the live panel sit in. */
  const cardStyle = (accent: string) => ({
    position: 'fixed' as const, right: 16, bottom: 16, zIndex: 999,
    background: gradient.dropdown, backdropFilter: 'blur(16px)',
    border: `1px solid ${accent}`, borderRadius: radius.lg,
    boxShadow: '0 12px 40px rgba(0,0,0,0.6)',
    padding: '10px 14px', fontFamily: font.body,
    display: 'flex', flexDirection: 'column' as const, gap: 8,
  });

  /** What the panel can honestly claim about capture right now. */
  const captureLine = () => {
    const near = nearChunksHeard > 0 ? 'hearing you ✓' : 'listening…';
    if (!systemAudio) return `Your voice — ${near}`;
    const far = farChunksHeard > 0 ? 'hearing the call ✓' : 'waiting for call audio…';
    return `Both sides — you: ${near} · call: ${far}`;
  };

  const newestDraft = recoveredDrafts[0];

  return (
    <>
      {/* Sidebar row (mirrors SidebarRow styling; red while recording). The
          hover it never had now comes from `.pa-btn`, matching SidebarRow's
          (lift toward the selected look) rather than inventing a third one.
          `` dissolves the primitive's label wrapper
          so the icon, the text and the REC beacon stay direct flex children of
          the row, exactly as before. `transition` stays inline and stays
          `all`: the rail's collapse animates width, padding and margin, which
          `.pa-btn`'s own transition list does not cover. */}
      <Button
        colors={colors}
        variant="bare"
        onClick={onButton}
        title={
          state === 'recording' ? 'Stop recording and save the note'
            : state === 'finishing' ? 'Transcribing & saving…'
            : 'Record a meeting to a project note'
        }
        aria-label="Record meeting"
                style={{
          '--pa-btn-bg': state === 'recording' ? colors.danger + '24' : active ? colors.cyanSoft : 'transparent',
          '--pa-btn-fg': state === 'recording' ? colors.danger : active ? colors.cyan : colors.textMuted,
          '--pa-btn-border': state === 'recording' ? colors.danger : active ? colors.borderHi : 'transparent',
          '--pa-btn-bg-hover': state === 'recording' ? colors.danger + '24' : active ? colors.cyanSoft : colors.borderHi,
          '--pa-btn-border-hover': state === 'recording' ? colors.danger : active ? colors.borderHi : 'transparent',
          '--pa-btn-fg-hover': state === 'recording' ? colors.danger : colors.cyan,
          '--pa-btn-bg-active': state === 'recording' ? colors.danger + '24' : active ? colors.cyanSoft : colors.borderHi,
          '--pa-btn-pad': open ? '0 12px' : '0',
          '--pa-btn-radius': '10px',
          '--pa-btn-weight': active ? 600 : 500,
          width: open ? 'calc(100% - 16px)' : 40,
          height: 40,
          gap: 12,
          justifyContent: open ? 'flex-start' : 'center',
          margin: open ? '0 8px' : '0 auto',
          cursor: state === 'finishing' ? 'default' : 'pointer',
          transition: `all 200ms ${ease.out}`,
          fontFamily: font.body, fontSize: 13,
          textAlign: 'left',
        } as CSSProperties}
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
        {/* Collapsed-rail REC beacon: the rail is real DOM outside <main>, so
            unlike a floating chip it can never be buried by the browser. */}
        {!open && state === 'recording' && (
          <span className="pa-rec-dot" style={{
            position: 'absolute', top: 5, right: 5, width: 7, height: 7,
            borderRadius: '50%', background: colors.danger,
            animation: 'pa-rec-pulse 1.4s ease-in-out infinite',
          }} />
        )}
      </Button>

      {/* Confirm-first project picker. */}
      {pickerOpen && createPortal(
        <div
          onMouseDown={e => { if (e.target === e.currentTarget) { setPickerOpen(false); setFilter(''); } }}
          style={{
            position: 'fixed', inset: 0, zIndex: 1000,
            background: 'rgba(0,0,0,0.45)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-label="Record a meeting"
            style={{
              width: 380, maxWidth: 'calc(100vw - 48px)', maxHeight: '70vh',
              display: 'flex', flexDirection: 'column',
              background: gradient.dropdown, backdropFilter: 'blur(16px)',
              border: `1px solid ${colors.borderHi}`, borderRadius: radius.lg,
              boxShadow: '0 12px 40px rgba(0,0,0,0.6)',
              padding: 16, fontFamily: font.body,
            }}
          >
            <div style={{ fontSize: 14, fontWeight: 600, color: colors.text, marginBottom: 4 }}>
              Record a meeting
            </div>
            <div style={{ fontSize: 11, color: colors.textMuted, lineHeight: 1.5, marginBottom: 10 }}>
              {systemAudio
                ? "Records BOTH sides — your microphone and this Mac's audio output, which is the other participants. The audio is transcribed on this device and never uploaded. Saved as a note on the project you pick."
                : "Records your own voice from this machine's microphone (not the other side of a call), transcribes it on this device — the audio is never uploaded — and saves the transcript as a note on the project you pick when you stop."}
            </div>

            {/* The transcript is written up by the configured model, and that
                is a separate question from where the audio was transcribed.
                Saying only "transcribed locally" here read as a blanket privacy
                guarantee and was not one: the write-up sends the transcript
                text to whichever provider the user has configured, which is a
                cloud provider unless they have set a local one. A user
                recording a private meeting deserves to be told that at the
                moment they record it, not to find it in an egress log. */}
            {/* Stated unconditionally rather than computed. The signals the UI
                can cheaply reach — `localProviderAvailable` from the
                sovereignty endpoint — say a local provider EXISTS, not that it
                is the configured one, and a privacy line that is right most of
                the time is worse than one that is always right. */}
            <div style={{ fontSize: 11, color: colors.textMuted, lineHeight: 1.5, marginBottom: 10 }}>
              Afterwards the transcript text is sent to your configured model to
              write the summary and pull out to-dos. Configure a local model, or
              turn on Sovereign mode, to keep the text on this device too.
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
                <Button
                  colors={colors}
                  variant="bare"
                  onClick={loadProjects}
                  style={{
                    '--pa-btn-fg': colors.cyan,
                    '--pa-btn-bg-hover': 'transparent',
                    '--pa-btn-pad': '0',
                    '--pa-btn-weight': 600,
                    fontFamily: font.body, fontSize: 11,
                  } as CSSProperties}
                >Retry</Button>
              </div>
            )}
            {!loadError && projects === null && (
              <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 8 }}>Loading projects…</div>
            )}

            {projects !== null && projects.length > FILTER_THRESHOLD && (
              <input
                ref={filterRef}
                value={filter}
                onChange={e => setFilter(e.target.value)}
                placeholder="Filter projects…"
                aria-label="Filter projects"
                style={{
                  marginBottom: 8, padding: '6px 8px', borderRadius: 7,
                  background: colors.inputBg, color: colors.text,
                  border: `1px solid ${colors.border}`, outline: 'none',
                  fontFamily: font.body, fontSize: 12,
                }}
              />
            )}

            {projects !== null && (
              <div style={{ overflow: 'auto', flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', gap: 4, marginBottom: 12 }}>
                {visibleProjects.length === 0 && (
                  <div style={{ fontSize: 11, color: colors.textDim }}>
                    {projects.length === 0 ? 'No projects available.' : 'No project matches that.'}
                  </div>
                )}
                {visibleProjects.map(p => (
                  <Button
                    key={p.id}
                    colors={colors}
                    onClick={() => setSelectedId(p.id)}
                    onDoubleClick={() => { setSelectedId(p.id); void handleStart(); }}
                    aria-pressed={selectedId === p.id}
                    style={{
                      '--pa-btn-bg': selectedId === p.id ? colors.cyanSoft : 'transparent',
                      '--pa-btn-fg': selectedId === p.id ? colors.cyan : colors.text,
                      '--pa-btn-border': selectedId === p.id ? colors.borderHi : colors.border,
                      '--pa-btn-bg-hover': selectedId === p.id ? colors.cyanSoft : colors.surfaceHi,
                      '--pa-btn-border-hover': selectedId === p.id ? colors.borderHi : colors.borderHi,
                      '--pa-btn-fg-hover': selectedId === p.id ? colors.cyan : colors.text,
                      '--pa-btn-bg-active': selectedId === p.id ? colors.cyanSoft : colors.surface,
                      '--pa-btn-pad': '8px 10px',
                      '--pa-btn-radius': `${radius.md}px`,
                      '--pa-btn-weight': selectedId === p.id ? 600 : 400,
                      // The list is a column of full-width rows: the name reads
                      // from the left edge, not from the middle of the row.
                      justifyContent: 'flex-start',
                      textAlign: 'left', fontSize: 12, fontFamily: font.body,
                    } as CSSProperties}
                  >
                    {p.name}
                  </Button>
                ))}
              </div>
            )}

            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', alignItems: 'center' }}>
              <span style={{ fontSize: 10, color: colors.textDim, marginRight: 'auto' }}>
                Esc to cancel · Enter to start
              </span>
              <Button
                colors={colors}
                onClick={() => { setPickerOpen(false); setFilter(''); }}
                style={{
                  '--pa-btn-bg': 'transparent',
                  '--pa-btn-fg': colors.textMuted,
                  '--pa-btn-border': colors.border,
                  '--pa-btn-bg-hover': colors.surfaceHi,
                  '--pa-btn-border-hover': colors.borderHi,
                  '--pa-btn-fg-hover': colors.text,
                  '--pa-btn-pad': '7px 14px',
                  '--pa-btn-radius': '7px',
                  fontFamily: font.body, fontSize: 12,
                } as CSSProperties}
              >
                Cancel
              </Button>
              <Button
                colors={colors}
                onClick={handleStart}
                disabled={!selectedId}
                style={{
                  '--pa-btn-bg': colors.cyanSoft,
                  '--pa-btn-fg': colors.cyan,
                  '--pa-btn-border': colors.borderHi,
                  '--pa-btn-bg-hover': colors.cyanSoft,
                  '--pa-btn-border-hover': colors.cyan,
                  '--pa-btn-bg-active': colors.cyanSoft,
                  '--pa-btn-pad': '7px 14px',
                  '--pa-btn-radius': '7px',
                  '--pa-btn-weight': 600,
                  fontFamily: font.body, fontSize: 12,
                } as CSSProperties}
              >
                Start recording
              </Button>
            </div>
          </div>
        </div>,
        document.body,
      )}

      {/* A transcript stranded by a crash/quit mid-recording: offer it back
          before it can be forgotten. One card at a time, newest first, so a
          stack of interrupted meetings is worked through rather than buried.
          Renders only while idle so it never competes with a live recording. */}
      {newestDraft && state === 'idle' && createPortal(
        <div style={{ ...cardStyle(colors.borderHi), maxWidth: 340 }}>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
            <div style={{ fontSize: 12, fontWeight: 600, color: colors.text }}>
              Interrupted recording recovered
            </div>
            {recoveredDrafts.length > 1 && (
              <span style={{ fontSize: 10, color: colors.textDim }}>
                1 of {recoveredDrafts.length}
              </span>
            )}
          </div>
          <div style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
            A recording for "{newestDraft.projectName}" was cut off before it was
            saved. The transcribed part survived — save it as the meeting note,
            or let it go.
          </div>
          {/* A failed recovery save used to set an error that nothing on screen
              could render: the button simply sprang back to "Save the
              transcript" and the user had no idea why. */}
          {error && (
            <div style={{ fontSize: 11, color: colors.danger, lineHeight: 1.4 }}>{error}</div>
          )}
          <div style={{ display: 'flex', gap: 8 }}>
            <Button
              colors={colors}
              onClick={async () => {
                setRecovering(newestDraft.startedAt);
                const saved = await recoverDraft(newestDraft);
                setRecovering(null);
                if (saved) savedToast(`The recovered transcript was saved as a note on "${newestDraft.projectName}" — click to open it.`, saved);
                // `recoverDraft` reports failure by returning null (and setting
                // `error`, rendered above) — never tick on that.
                return saved !== null;
              }}
              disabled={recovering === newestDraft.startedAt}
              style={{ ...btnVars('primary'), cursor: recovering ? 'default' : 'pointer' } as CSSProperties}
            >
              {recovering === newestDraft.startedAt ? 'Saving…' : 'Save the transcript'}
            </Button>
            <Button colors={colors} onClick={() => dismissDraft(newestDraft)} style={btnVars('quiet')}>
              Discard
            </Button>
          </div>
        </div>,
        document.body,
      )}

      {/* Always-visible recording / saving / error panel. Docks into the chat
          sidebar's slot when the dock is open; floats bottom-right otherwise. */}
      {(busy || state === 'error') && createPortal(
        <div style={{
          ...cardStyle(state === 'recording' ? colors.danger : colors.borderHi),
          width: state === 'recording' ? panelWidth : 340,
          maxWidth: 'calc(100vw - 32px)',
          transition: `width 220ms ${ease.out}`,
          ...(docked ? {
            position: 'relative' as const, right: 'auto', bottom: 'auto', zIndex: 1,
            width: '100%', maxWidth: '100%', boxShadow: 'none', borderRadius: 10,
          } : null),
        }}>
          <style>{
            '@keyframes pa-rec-pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.25; } }' +
            '@media (prefers-reduced-motion: reduce) { .pa-rec-dot { animation: none !important; } }'
          }</style>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            {state === 'recording' && (
              <span className="pa-rec-dot" style={{
                width: 9, height: 9, borderRadius: '50%', background: colors.danger,
                animation: 'pa-rec-pulse 1.4s ease-in-out infinite', flexShrink: 0,
              }} />
            )}
            <span style={{ fontSize: 12, fontWeight: 600, color: state === 'error' ? colors.danger : colors.text }}>
              {state === 'recording' && `Recording ${formatElapsed(elapsedSeconds)}`}
              {state === 'finishing' && 'Transcribing & writing your notes…'}
              {state === 'error' && 'Meeting dictation'}
            </span>
            {target && (
              <span style={{ fontSize: 11, color: colors.textDim, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                → {target.projectName}
              </span>
            )}
            {state === 'recording' && !docked && (
              <Button
                colors={colors}
                variant="bare"
                onClick={() => setExpanded(v => !v)}
                aria-label={expanded ? 'Collapse the notepad' : 'Expand the notepad'}
                title={expanded ? 'Collapse the notepad' : 'Give the notepad more room'}
                style={{
                  '--pa-btn-fg': colors.textMuted,
                  '--pa-btn-fg-hover': colors.text,
                  '--pa-btn-pad': '2px',
                  '--pa-btn-radius': `${radius.xs}px`,
                  marginLeft: 'auto',
                } as CSSProperties}
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                  strokeWidth={2} strokeLinecap="round" strokeLinejoin="round"
                  style={{ transform: expanded ? 'rotate(180deg)' : 'none', transition: `transform 200ms ${ease.out}` }}>
                  <path d={CHEVRON} />
                </svg>
              </Button>
            )}
          </div>

          {state === 'recording' && (
            <div style={{ fontSize: 10, color: colors.textDim }}>
              {captureLine()}
              {' · transcribed locally on this device'}
              {failedChunks > 0 && ` · ${failedChunks} segment${failedChunks > 1 ? 's' : ''} failed to transcribe`}
            </div>
          )}

          {/* The notepad. Granola's core insight: while the meeting runs the
              surface is the user's own cursor, not the machine's output — and
              what they jot STEERS the summary rather than merely bookmarking
              it. Sparse by design; empty is fine and changes nothing. */}
          {state === 'recording' && (
            <>
              <textarea
                ref={notepadRef}
                value={userNotes}
                onChange={e => setUserNotes(e.target.value)}
                placeholder="Jot what matters — I'll build the notes around it"
                aria-label="Meeting notepad"
                style={{
                  width: '100%', resize: 'vertical',
                  height: expanded ? 260 : 96,
                  background: colors.inputBg, color: colors.text,
                  border: `1px solid ${colors.border}`, borderRadius: radius.md,
                  padding: '8px 10px', fontFamily: font.body, fontSize: 12,
                  lineHeight: 1.6, outline: 'none',
                  transition: `height 220ms ${ease.out}`,
                }}
              />
              <div style={{ fontSize: 10, color: colors.textDim }}>
                {userNotes.trim()
                  ? 'Your notes stay verbatim and steer the write-up · saved as you type'
                  : 'Optional — the transcript is captured either way'}
              </div>
            </>
          )}

          {/* A far-side failure mid-recording (e.g. Screen Recording permission
              missing) used to be set only where the closed picker would have
              shown it — the user recorded a whole call believing both sides
              were captured. Surface it here, where they are looking. */}
          {state === 'recording' && systemAudioError && (
            <div style={{ fontSize: 11, color: colors.danger, lineHeight: 1.4 }}>{systemAudioError}</div>
          )}

          {error && (
            <div style={{ fontSize: 11, color: colors.danger, lineHeight: 1.4 }}>{error}</div>
          )}

          <div style={{ display: 'flex', gap: 8 }}>
            {state === 'recording' && (
              <>
                {/* Stopping is confirmed by the panel changing to "Transcribing
                    & writing your notes…" and then going away — a tick on top
                    of that would be a second, later claim about the same act. */}
                <Button colors={colors} onClick={handleStop} flashSuccess={false} style={btnVars('primary')}>
                  Stop &amp; save
                </Button>
                <Button
                  colors={colors}
                  onClick={() => (confirmDiscard ? discard() : setConfirmDiscard(true))}
                  style={confirmDiscard ? btnVars('danger') : btnVars('quiet')}
                >
                  {confirmDiscard ? 'Discard — sure?' : 'Discard'}
                </Button>
              </>
            )}
            {state === 'error' && (
              <>
                {hasUnsavedTranscript && (
                  <Button colors={colors} onClick={handleRetry} style={btnVars('primary')}>Retry save</Button>
                )}
                <Button colors={colors} onClick={discard} style={btnVars('quiet')}>Dismiss</Button>
              </>
            )}
          </div>
        </div>,
        dockSlot ?? document.body,
      )}
    </>
  );
}
