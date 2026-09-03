// Grow — the tab that follows Build. Build makes the thing; Grow takes it to
// market. Per-project GTM home: the five-pillar strategy canvas, a content
// calendar of social posts, and Henry-driven growth actions. Henry knows the
// project (Brain, people, docs, goals), so Grow drafts and schedules with
// real context. Publishing goes through Postiz (Cloud or self-hosted) as a
// separate HTTP publisher — this repo does not vendor Postiz. Each project
// connects its own Instagram / LinkedIn / X login.


import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { ViewHeader } from '../common/ViewHeader';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';
import type { Project } from '../projects/types';
import type { SocialCard } from './calendarPosts';
import { SEGMENT_STRIP_PAD, SEGMENT_STRIP_RADIUS, segmentedTab } from './growStyles';
import {
  FIELD_CLASS,
  GrowChrome,
  LINK_CLASS,
  growChromeVars,
  growField,
} from './growChrome';
import { ErrorState, LoadingState } from './GrowStateBlocks';
import { GrowResults } from './GrowResults';
import { GrowActions } from './GrowActions';
import { GrowAnalytics } from './GrowAnalytics';
import { StrategyLens } from './StrategyLens';
import { CalendarSection } from './CalendarLens';
import type { GrowLens, LoadState } from './growTypes';

/** How long the panel takes to fade out before the project actually changes,
 *  and to fade back in after. Long enough to read as a transition, short
 *  enough that switching never feels like waiting. */
const SWAP_FADE_MS = 140;
/** How long the outgoing height stays pinned after the swap, so a panel that
 *  is still fetching cannot collapse the scroll container under the cursor. */
const SWAP_SETTLE_MS = 600;


// Actions leads: the point of collecting analytics is deciding what to do.
// Results sits next to it so "what I did" is as reachable as "what to do".
const LENSES: GrowLens[] = ['actions', 'results', 'strategy', 'calendar', 'analytics'];
const LENS_LABELS: Record<GrowLens, string> = {
  actions: 'Actions',
  results: 'Results',
  strategy: 'Strategy',
  calendar: 'Calendar',
  analytics: 'Analytics',
};


export function GrowView() {
  const { colors, gradient, reduceMotion } = useTheme();
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectsState, setProjectsState] = useState<LoadState>('loading');
  // Shared with Projects and Build (J7): opening a project in one of them
  // points this one at the same project, and vice versa. Grow used to forget
  // its project on every mount; now it remembers the app's.
  const activeId = useCommandCenter((st) => st.currentProjectId);
  const setActiveId = useCommandCenter((st) => st.setCurrentProject);
  const [posts, setPosts] = useState<SocialCard[]>([]);
  const [postsState, setPostsState] = useState<LoadState>('loading');
  const [postsMutationError, setPostsMutationError] = useState<string | null>(null);
  const [lens, setLens] = useState<GrowLens>('actions');
  const [ctx, setCtx] = useState<{ people: number; goals: number } | null>(null);
  const [focusLens, setFocusLens] = useState<GrowLens | null>(null);
  const postsRequestGeneration = useRef(0);
  // Project switching. Every panel refetches at once, so a bare `setActiveId`
  // is a hard cut: the whole column drops to its loading states in one frame
  // and springs back when the slowest request lands. Fading out BEFORE the
  // switch — which we can do because we own the trigger — means the swap and
  // the loading states happen while nothing is visible, and the new project
  // arrives as one smooth rise instead of a flash.
  const [swapping, setSwapping] = useState(false);
  // What the user has chosen but the panel has not caught up to yet. The
  // dropdown is bound to this, not to `activeId` — a control that springs back
  // to the old value for the length of the fade reads as the app arguing with
  // the click, which is worse than the flash we came here to remove.
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [pinnedHeight, setPinnedHeight] = useState<number | undefined>(undefined);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const swapTimer = useRef<ReturnType<typeof setTimeout>>();
  const setActivePanel = useCommandCenter((st) => st.setActivePanel);
  const sendMessage = useCommandCenter((st) => st.sendMessage);
  const openChatDock = useCommandCenter((st) => st.openChatDock);
  const openGrowForProject = useCommandCenter((st) => st.openGrowForProject);
  const setOpenGrowForProject = useCommandCenter((st) => st.setOpenGrowForProject);
  const openGrowLens = useCommandCenter((st) => st.openGrowLens);
  const setOpenGrowLens = useCommandCenter((st) => st.setOpenGrowLens);
  const setPendingProjectNavigation = useCommandCenter((st) => st.setPendingProjectNavigation);

  const loadProjects = useCallback(() => {
    setProjectsState('loading');
    apiFetch<Project[]>('/api/projects')
      .then((ps) => {
        const real = ps.filter((p) => p.status !== 'archived');
        setProjects(real);
        setProjectsState('ready');
      })
      .catch(() => setProjectsState('error'));
  }, []);

  // projectsRev bumps on project_changed — a strategy save (from the UI or
  // Henry's set_project_strategy tool) refreshes the cards live.
  const projectsRev = useCommandCenter((st) => st.projectsRev);
  // The primary agent's configured display name — identity is config, never a literal (#986).
  const agentName = useCommandCenter((st) => st.agentName);
  useEffect(() => { loadProjects(); }, [loadProjects, projectsRev]);

  // Content calendar = social_post cards on this project (reserved card type
  // already exists; empty until Henry/the user create them).
  const loadPosts = useCallback((id: string, opts?: { silent?: boolean }) => {
    const generation = ++postsRequestGeneration.current;
    // Background refreshes keep the current list on screen (no loading flash);
    // only user-visible (re)loads show the loading state.
    if (!opts?.silent) setPostsState('loading');
    apiFetch<SocialCard[]>(`/api/projects/${encodeURIComponent(id)}/cards?card_type=social_post`)
      .then((p) => {
        if (generation !== postsRequestGeneration.current) return;
        setPosts(p);
        setPostsState('ready');
      })
      .catch(() => {
        if (generation !== postsRequestGeneration.current) return;
        if (opts?.silent) {
          // Keep a list that's already on screen. If this was racing the
          // first load, do not leave the calendar stuck on "Loading posts…".
          setPostsState((s) => (s === 'loading' ? 'error' : s));
          return;
        }
        setPosts([]);
        setPostsState('error');
      });
  }, []);

  useEffect(() => {
    if (!activeId) return;
    loadPosts(activeId);
    return () => { ++postsRequestGeneration.current; };
  }, [activeId, loadPosts]);

  // project_changed (brand/strategy save, media job finished) refreshes
  // the calendar without a loading flash. Skip the initial stamp — the
  // activeId effect already loaded.
  const seenPostsRev = useRef(projectsRev);
  useEffect(() => {
    if (!activeId) return;
    if (seenPostsRev.current === projectsRev) return;
    seenPostsRev.current = projectsRev;
    loadPosts(activeId, { silent: true });
  }, [projectsRev, activeId, loadPosts]);

  // PATCH/DELETE /api/projects/:id/cards/:cardId — confirmed paths in routes/cards.rs
  // (patch + delete on the same resource; there is no PUT /api/cards/:id).
  const mutatePost = useCallback(async (
    projectId: string,
    post: SocialCard,
    body: Record<string, unknown> | null,
  ) => {
    setPostsMutationError(null);
    const path = `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(post.id)}`;
    try {
      if (body === null) {
        await apiFetch(path, { method: 'DELETE' });
      } else {
        await apiFetch(path, {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        });
      }
      loadPosts(projectId, { silent: true });
    } catch (e) {
      const msg = e instanceof Error ? e.message : 'Could not update the post.';
      setPostsMutationError(msg);
      throw e;
    }
  }, [loadPosts]);

  // Keep the Content calendar live while it's on screen: "+ Draft a post with
  // Henry" hands off to chat, and before this poll the drafted social_post
  // card never appeared until the user switched projects and back (2026-07
  // wiring audit — persist-but-no-readback). Same 15s stale-while-revalidate
  // cadence the dashboard uses.
  useEffect(() => {
    if (!activeId || lens !== 'calendar') return;
    const t = setInterval(() => loadPosts(activeId, { silent: true }), 15_000);
    return () => clearInterval(t);
  }, [activeId, lens, loadPosts]);

  const active = projects.find((p) => p.id === activeId) ?? null;

  // Grow needs *a* project to show, and the shared selection starts empty on a
  // first run. Adopt the first one only when nothing at all is selected — NOT
  // when the selection simply isn't in this list. Grow hides archived
  // projects; silently re-pointing the shared selection because of that would
  // yank the Projects tab off the board the user deliberately opened.
  useEffect(() => {
    if (activeId || projects.length === 0) return;
    setActiveId(projects[0].id);
  }, [activeId, projects, setActiveId]);

  /** Every project change goes through here — the dropdown and the cross-tab
   *  deep link alike, so one of them can never feel different from the other. */
  const switchProject = useCallback((id: string) => {
    if (!id || id === activeId) return;
    clearTimeout(swapTimer.current);
    if (reduceMotion) { setActiveId(id); return; }
    // Hold the height we are leaving so the scroll container cannot lurch
    // while the new panel is still empty.
    setPinnedHeight(panelRef.current?.offsetHeight);
    setPendingId(id);
    setSwapping(true);
    swapTimer.current = setTimeout(() => {
      setActiveId(id);
      setPendingId(null);
      setSwapping(false);
      // Release the pin once the new content has had time to lay out.
      swapTimer.current = setTimeout(() => setPinnedHeight(undefined), SWAP_SETTLE_MS);
    }, SWAP_FADE_MS);
  }, [activeId, reduceMotion, setActiveId]);

  useEffect(() => () => clearTimeout(swapTimer.current), []);

  // Honor a cross-tab deep link (Projects → Grow this project), then CLEAR it
  // (the pendingProjectNavigation consume-then-clear pattern). Without the
  // clear, one agent-driven grow open stuck in the store forever: every later
  // manual Grow visit re-selected that project on remount, and a repeat
  // open for the same project was a silent no-op (same value → no re-render).
  useEffect(() => {
    if (openGrowForProject) {
      switchProject(openGrowForProject);
      setOpenGrowForProject(null);
    }
  }, [openGrowForProject, setOpenGrowForProject, switchProject]);

  useEffect(() => {
    if (!openGrowLens) return;
    setLens(openGrowLens);
    setOpenGrowLens(null);
  }, [openGrowLens, setOpenGrowLens]);

  // Real project context — Grow feels connected because it shows the project's
  // actual state (people, shipped work), not a blank canvas.
  useEffect(() => {
    if (!activeId) { setCtx(null); return; }
    let alive = true;
    (async () => {
      const [people, cards] = await Promise.all([
        apiFetch<unknown[]>(`/api/projects/${encodeURIComponent(activeId)}/people`).catch(() => []),
        apiFetch<{ card_type: string }[]>(`/api/projects/${encodeURIComponent(activeId)}/cards`).catch(() => []),
      ]);
      if (!alive) return;
      // Count of goal cards in ANY state — labeled "goals", not "shipped"
      // (2026-07 wiring audit: the old "N shipped" label counted in-progress
      // and triage cards as shipped work).
      const goals = cards.filter((c) => c.card_type === 'goal').length;
      setCtx({ people: people.length, goals });
    })();
    return () => { alive = false; };
  }, [activeId]);

  // One-click hand-off: surface chat and send the GTM prompt directly to Henry,
  // grounded in the selected project (the Discuss-with-Henry pattern). No
  // clipboard, no tab hunting.
  const send = (prompt: string) => {
    // Open the chat dock explicitly. setActivePanel('chat') only dismisses any
    // overlay — since chat went dock-first it does NOT surface Henry, so these
    // cards looked dead: the prompt was sent to a chat nobody could see.
    setActivePanel('chat');
    openChatDock();
    void sendMessage(prompt);
  };

  // Close the one-way door: Projects → Grow deep-links in (openGrowForProject),
  // but Grow had no way back. Return to this project in the Projects tab, reusing
  // the pendingProjectNavigation seam ProjectsView consumes (mirrors BrainView's
  // "Open project"). No new store seam.
  const openInProjects = useCallback(() => {
    if (!activeId) return;
    setPendingProjectNavigation(activeId);
    navigateToTool('projects');
  }, [activeId, setPendingProjectNavigation]);

  return (
    <div
      data-grow-chrome=""
      style={{
        width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
        background: gradient.workspace, color: colors.text, fontFamily: font.body,
        ...growChromeVars(colors),
      }}
    >
      <GrowChrome />
      {/* Header + project switcher, from ViewHeader so Grow wears the same
          header as Home, Projects, Automate and Build.

          THE HARD SCROLL EDGE (D11): `surface={colors.bg}` is ViewHeader's
          own opaque fill for this (see `hardScrollEdgeSurface` in
          ViewHeader.tsx). The header used to be transparent over the
          workspace gradient, so the panel's first card showed faintly
          through it and the boundary was a hairline over a see-through bar;
          `surface` plus that one hairline IS the hard edge — one per view.

          The brand ribbon that used to sit on the old wrapper div is gone: a
          2px cyan-to-purple gradient across the header is decoration
          standing in for hierarchy (D13) and a second tint competing with
          the view's one accented action (D8). It said nothing the title
          does not. */}
      <ViewHeader
        surface={colors.bg}
        title="Grow"
        subtitle={
          <span style={{ display: 'flex', alignItems: 'center', gap: space.lg, flexWrap: 'wrap' }}>
            <span>Take {active ? active.name : 'your project'} to market — {agentName} drafts with the project's real context.</span>
            {active?.siteUrl && (
              <a href={active.siteUrl} className={LINK_CLASS} target="_blank" rel="noreferrer" style={{ color: colors.cyan }}>site ↗</a>
            )}
            {active?.repoUrl && (
              <a href={active.repoUrl} className={LINK_CLASS} target="_blank" rel="noreferrer" style={{ color: colors.cyan }}>repo ↗</a>
            )}
            {active && (
              <Tooltip content={`Open ${active.name} in Projects`} placement="bottom">
              <Button
                colors={colors}
                variant="bare"
                type="button"
                onClick={openInProjects}
                style={{
                  '--pa-btn-fg': colors.cyan,
                  '--pa-btn-fg-hover': colors.cyan,
                  '--pa-btn-bg-hover': 'transparent',
                  '--pa-btn-pad': '0',
                  '--pa-btn-weight': 'inherit',
                  fontSize: 'inherit',
                  lineHeight: 'inherit',
                } as CSSProperties}
              >open project ↗</Button>
              </Tooltip>
            )}
            {/* Always rendered so the count fades in rather than popping the
                header line around on every project change. */}
            <span style={{
              color: colors.textDim,
              opacity: ctx ? 1 : 0,
              transition: reduceMotion ? undefined : `opacity ${duration.smooth}ms ${ease.smooth}`,
            }}>
              {ctx && `${ctx.goals} ${ctx.goals === 1 ? 'goal' : 'goals'} · ${ctx.people} ${ctx.people === 1 ? 'person' : 'people'}`}
            </span>
          </span>
        }
        actions={<>
      {/* VIEW axis — segmented tab toggle (mirrors the Kanban/overview toggle) */}
      {/* The strip is the container and each tab is its child, so the tab's
          radius is the strip's minus the strip's padding — `concentric()`,
          not a second number picked by eye (D4). `segmentedTab` derives it
          from the same two values. */}
      <div role="tablist" aria-label="Grow view" style={{ display: 'flex', gap: SEGMENT_STRIP_PAD, background: colors.bgDeeper, borderRadius: SEGMENT_STRIP_RADIUS, padding: SEGMENT_STRIP_PAD }}>
        {LENSES.map((l) => {
          const selected = lens === l;
          return (
            // `role="tab"` inside a tablist: keep the element and take the
            // shared `.pa-btn` interaction rules rather than the Button
            // component, which would flatten the tab semantics. Nothing here
            // is awaited, so pending/success would be wrong for it anyway.
            <button
              key={l}
              className="pa-btn"
              role="tab"
              aria-selected={selected}
              tabIndex={0}
              onClick={() => setLens(l)}
              onFocus={() => setFocusLens(l)}
              onBlur={() => setFocusLens(null)}
              style={{
                ...segmentedTab(colors, selected),
                boxShadow: focusLens === l ? `0 0 0 2px ${colors.borderHi}` : 'none',
              }}
            >{LENS_LABELS[l]}</button>
          );
        })}
      </div>
      <select
        value={pendingId ?? activeId ?? ''}
        onChange={(e) => switchProject(e.target.value)}
        aria-label="Select project"
        className={FIELD_CLASS}
        style={{ ...growField(colors), borderRadius: radius.md, fontSize: textSize.small }}
      >
        {/* The shared selection can point at a project Grow doesn't track.
            An unlisted value would render the switcher blank, which reads as
            "no project" rather than "not this one". */}
        {activeId && !active && <option value={activeId} disabled>Not tracked here</option>}
        {projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
      </select>
        </>}
      />

      {projectsState === 'error' ? (
        <ErrorState
          colors={colors}
          message="Couldn't load your projects."
          onRetry={loadProjects}
        />
      ) : projectsState === 'loading' && projects.length === 0 ? (
        <LoadingState colors={colors} label="Loading projects…" />
      ) : active ? (
        <div
          ref={panelRef}
          role="tabpanel"
          aria-label={`${lens} view`}
          aria-busy={swapping}
          style={{
            flex: 1, overflowY: 'auto', padding: `${space.xxxl}px ${space.huge}px`,
            display: 'flex', flexDirection: 'column', gap: space.xxxl,
            minHeight: pinnedHeight,
            opacity: swapping ? 0 : 1,
            transition: reduceMotion ? undefined : `opacity ${SWAP_FADE_MS}ms ${ease.smooth}`,
          }}
        >
          {/* Keyed for the same reason the analytics panels are: the load
              effect refetches on project.id but never clears local state, so
              without a remount project A's verify results and outcomes stay on
              screen over project B's cards for as long as the refetch is in
              flight. That leak was a reported bug on the analytics panels
              (2026-08-04) — see analyticsPanelScope.test.ts. */}
          {lens === 'actions' && <GrowActions key={active.id} project={active} colors={colors} />}
          {lens === 'results' && <GrowResults key={`${active.id}-results`} project={active} colors={colors} />}
          {lens === 'analytics' && <GrowAnalytics project={active} posts={posts} colors={colors} />}
          {lens === 'strategy' && (
            <StrategyLens active={active} colors={colors} send={send} agentName={agentName} />
          )}

          {lens === 'calendar' && (
            <CalendarSection
              active={active}
              colors={colors}
              agentName={agentName}
              send={send}
              posts={posts}
              postsState={postsState}
              postsMutationError={postsMutationError}
              onReload={(opts) => loadPosts(active.id, opts)}
              onMutate={mutatePost}
            />
          )}
        </div>
      ) : projects.length === 0 ? (
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: colors.textDim, fontSize: textSize.small }}>
          Create a project in the Projects tab, then grow it here.
        </div>
      ) : (
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: colors.textDim, fontSize: textSize.small, textAlign: 'center', padding: `0 ${space.huge}px`, lineHeight: 1.6 }}>
          The project you have open isn't tracked here — Grow skips archived
          projects. Pick one from the switcher above.
        </div>
      )}
    </div>
  );
}
