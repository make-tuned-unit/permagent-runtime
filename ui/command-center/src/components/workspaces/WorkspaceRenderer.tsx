import { useCallback, lazy, Suspense } from 'react';
import { Panel, Group, Separator } from 'react-resizable-panels';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { LayoutNode, LayoutSplit, LayoutPanel, ToolType } from '../../lib/store';
import { ChatView } from '../chat/ChatView';
import { SkillsPanel } from '../skills/SkillsPanel';
import { TerminalManager } from '../terminal/TerminalManager';
import { Browser } from '../browser';
const LazyWorldView = lazy(() => import('../world/WorldView').then(m => ({ default: m.WorldView })));
import { ExecutionTrace } from '../trace/ExecutionTrace';
import { BrainView } from '../brain/BrainView';
import { ErrorBoundary } from '../common/ErrorBoundary';
import { Dashboard } from '../dashboard/Dashboard';
import { BuildView } from '../build/BuildView';
import { GrowView } from '../grow/GrowView';
import { AutomateView } from '../automate/AutomateView';
import { ProjectsView } from '../projects/ProjectsView';
import { FinancierView } from '../financier/FinancierView';

const TOOL_COMPONENTS: Record<ToolType, React.ComponentType> = {
  chat: ChatView,
  skills: SkillsPanel,
  trace: ExecutionTrace,
  world: LazyWorldView,
  terminal: TerminalManager,
  browser: Browser,
  memory: BrainView,
  dashboard: Dashboard,
  build: BuildView,
  grow: GrowView,
  automate: AutomateView,
  projects: ProjectsView,
  financier: FinancierView,
};

function LayoutNodeRenderer({
  node,
  path,
  onResize,
}: {
  node: LayoutNode;
  path: string;
  onResize: (path: string, sizes: number[]) => void;
}) {
  const { colors } = useTheme();
  if (node.type === 'panel') {
    const panel = node as LayoutPanel;
    const Component = TOOL_COMPONENTS[panel.tool];
    if (!Component) {
      return (
        <div className="flex h-full items-center justify-center text-xs" style={{ fontFamily: font.body, color: colors.textMuted }}>
          Unknown tool: {panel.tool}
        </div>
      );
    }
    return (
      <ErrorBoundary surface={panel.tool}>
        <Suspense fallback={<div className="flex h-full items-center justify-center text-xs" style={{ fontFamily: font.body, color: colors.textMuted }}>Loading...</div>}>
          <Component />
        </Suspense>
      </ErrorBoundary>
    );
  }

  const split = node as LayoutSplit;
  const orientation = split.direction === 'horizontal' ? 'horizontal' : 'vertical';

  const handleLayoutChanged = useCallback(
    (layout: Record<string, number>) => {
      const values = Object.values(layout);
      if (values.length > 0) {
        onResize(path, values);
      }
    },
    [path, onResize]
  );

  return (
    <Group orientation={orientation} onLayoutChanged={handleLayoutChanged}>
      {split.children.map((child, i) => {
        const panelId = `${path}-${i}`;
        return (
          <LayoutChildPanel
            key={panelId}
            child={child}
            index={i}
            panelId={panelId}
            path={path}
            orientation={orientation}
            defaultSize={split.sizes[i]}
            onResize={onResize}
          />
        );
      })}
    </Group>
  );
}

function LayoutChildPanel({
  child,
  index,
  panelId,
  path,
  orientation,
  defaultSize,
  onResize,
}: {
  child: LayoutNode;
  index: number;
  panelId: string;
  path: string;
  orientation: 'horizontal' | 'vertical';
  defaultSize: number;
  onResize: (path: string, sizes: number[]) => void;
}) {
  const { colors } = useTheme();
  return (
    <>
      {index > 0 && (
        <Separator
          className={`relative flex items-center justify-center ${
            orientation === 'horizontal' ? 'w-1' : 'h-1'
          }`}
          onMouseEnter={e => { const d = e.currentTarget.firstElementChild as HTMLElement | null; if (d) d.style.backgroundColor = `${colors.cyan}80`; }}
          onMouseLeave={e => { const d = e.currentTarget.firstElementChild as HTMLElement | null; if (d) d.style.backgroundColor = colors.border; }}
          onMouseDown={e => { const d = e.currentTarget.firstElementChild as HTMLElement | null; if (d) d.style.backgroundColor = colors.cyan; }}
          onMouseUp={e => { const d = e.currentTarget.firstElementChild as HTMLElement | null; if (d) d.style.backgroundColor = `${colors.cyan}80`; }}
        >
          <div
            className={`transition-colors ${
              orientation === 'horizontal' ? 'w-px h-full' : 'h-px w-full'
            }`}
            style={{ backgroundColor: colors.border }}
          />
        </Separator>
      )}
      <Panel id={panelId} defaultSize={defaultSize} minSize={10}>
        <div className="h-full w-full overflow-hidden">
          <LayoutNodeRenderer
            node={child}
            path={`${path}.${index}`}
            onResize={onResize}
          />
        </div>
      </Panel>
    </>
  );
}

export function WorkspaceRenderer({ workspaceId }: { workspaceId: string }) {
  const { colors } = useTheme();
  const workspace = useCommandCenter(s =>
    s.workspaces.find(w => w.id === workspaceId)
  );
  const updateLayout = useCommandCenter(s => s.updateWorkspaceLayout);

  const handleResize = useCallback(
    (path: string, sizes: number[]) => {
      if (!workspace) return;
      const updated = updateSizesAtPath(workspace.layoutJson, path, sizes);
      if (updated) {
        updateLayout(workspaceId, updated);
      }
    },
    [workspace, workspaceId, updateLayout]
  );

  if (!workspace) {
    return (
      <div className="flex h-full items-center justify-center" style={{ fontFamily: font.body, color: colors.textMuted }}>
        Workspace not found
      </div>
    );
  }

  return (
    <div className="h-full w-full">
      <LayoutNodeRenderer
        node={workspace.layoutJson}
        path="root"
        onResize={handleResize}
      />
    </div>
  );
}

function updateSizesAtPath(
  node: LayoutNode,
  path: string,
  newSizes: number[]
): LayoutNode | null {
  if (path === 'root' && node.type === 'split') {
    return { ...node, sizes: newSizes };
  }

  if (node.type !== 'split') return null;

  const parts = path.split('.');
  if (parts[0] !== 'root' || parts.length < 2) return null;

  const childIndex = parseInt(parts[1], 10);
  if (isNaN(childIndex) || childIndex >= node.children.length) return null;

  if (parts.length === 2) {
    const child = node.children[childIndex];
    if (child.type === 'split') {
      const newChildren = [...node.children];
      newChildren[childIndex] = { ...child, sizes: newSizes };
      return { ...node, children: newChildren };
    }
    return null;
  }

  const remainingPath = 'root.' + parts.slice(2).join('.');
  const child = node.children[childIndex];
  const updatedChild = updateSizesAtPath(child, remainingPath, newSizes);
  if (!updatedChild) return null;

  const newChildren = [...node.children];
  newChildren[childIndex] = updatedChild;
  return { ...node, children: newChildren };
}
