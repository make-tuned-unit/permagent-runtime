/**
 * AppShell — `project_selected` activity events carry the started session's id.
 *
 * "Starting a chat within a project" (the Sidebar's "new chat in project"
 * affordance, and the Projects surface's "start chat" affordance) used to emit
 * `project_selected` synchronously — BEFORE `createNewTab` had created (or
 * reused) a session, so `session_id` was always `null` even though a chat was
 * genuinely starting. This pins the fix: the emit now waits for
 * `createNewTab` to resolve and forwards its real session id.
 *
 * Heavy UI subtrees (Sidebar / TopBar / StatusBar / CreateProjectDialog /
 * SettingsModal / AppShellContent, useAppStartup) are stubbed so the test
 * exercises only AppShell's own `handleStartChatFromProject` /
 * `handleNewChatInProject` wiring. The ACP session-creation boundary
 * (`@/shared/api/acp`) and the Tauri `invoke` channel (project list / path
 * resolution / activity emit) are faked instead of touching a real daemon or
 * ACP subprocess.
 */
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fixtureProject = vi.hoisted(() => ({
  id: "proj-1",
  name: "Permagent",
  description: "",
  prompt: "",
  icon: "🚀",
  color: "#000000",
  preferredProvider: null,
  preferredModel: null,
  workingDirs: [],
  useWorktrees: false,
  order: 0,
  archivedAt: null,
  createdAt: "",
  updatedAt: "",
  artifactsDir: "",
}));

const invokeCalls = vi.hoisted(() => [] as Array<[string, unknown]>);

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    invokeCalls.push([cmd, args]);
    if (cmd === "list_projects") return [fixtureProject];
    if (cmd === "resolve_path") return { path: "/mock/cwd" };
    if (cmd === "emit_activity") return { accepted: true };
    return undefined;
  }),
}));

vi.mock("@/shared/api/acp", () => {
  let counter = 0;
  return {
    acpCreateSession: vi.fn(async () => ({
      sessionId: `acp-session-${++counter}`,
    })),
    acpPrepareSession: vi.fn(async () => "acp-session-prepared"),
    acpListSessions: vi.fn(async () => []),
    acpLoadSession: vi.fn(async () => {}),
    acpSetModel: vi.fn(async () => {}),
    discoverAcpProviders: vi.fn(async () => []),
  };
});

vi.mock("@/features/sidebar/ui/Sidebar", () => ({
  Sidebar: (props: { onNewChatInProject: (projectId: string) => void }) => (
    <button onClick={() => props.onNewChatInProject(fixtureProject.id)}>
      new-chat-in-project
    </button>
  ),
}));

vi.mock("./ui/TopBar", () => ({ TopBar: () => null }));
vi.mock("@/features/status/ui/StatusBar", () => ({ StatusBar: () => null }));
vi.mock("@/features/projects/ui/CreateProjectDialog", () => ({
  CreateProjectDialog: () => null,
}));
vi.mock("@/features/settings/ui/SettingsModal", () => ({
  SettingsModal: () => null,
}));
vi.mock("./hooks/useAppStartup", () => ({ useAppStartup: () => {} }));
vi.mock("./ui/AppShellContent", () => ({
  AppShellContent: (props: {
    onStartChatFromProject: (project: typeof fixtureProject) => void;
  }) => (
    <button onClick={() => props.onStartChatFromProject(fixtureProject)}>
      start-chat-from-project
    </button>
  ),
}));

import { AppShell } from "./AppShell";
import { useChatSessionStore } from "@/features/chat/stores/chatSessionStore";
import { useProjectStore } from "@/features/projects/stores/projectStore";

function emitCalls() {
  return invokeCalls.filter(([cmd]) => cmd === "emit_activity");
}

describe("AppShell — project_selected carries the started session's id", () => {
  beforeEach(() => {
    invokeCalls.length = 0;
    localStorage.clear();
    useChatSessionStore.setState({
      sessions: [],
      activeSessionId: null,
      isLoading: false,
      hasHydratedSessions: false,
      contextPanelOpenBySession: {},
      activeWorkspaceBySession: {},
    });
    useProjectStore.setState({
      projects: [fixtureProject],
      loading: false,
      activeProjectId: null,
    });
  });

  afterEach(() => {
    vi.clearAllMocks();
    invokeCalls.length = 0;
  });

  it('"start chat from project" (Projects surface) emits project_selected with the new session id, after the session exists', async () => {
    const user = userEvent.setup();
    render(<AppShell />);

    await user.click(
      screen.getByRole("button", { name: "start-chat-from-project" }),
    );

    await waitFor(() => expect(emitCalls().length).toBeGreaterThan(0));

    const [, args] = emitCalls()[emitCalls().length - 1];
    const payload = args as Record<string, unknown>;
    expect(payload.event_type).toBe("project_selected");
    expect(payload.session_id).toEqual(expect.stringMatching(/^acp-session-\d+$/));
    expect(payload.session_id).not.toBeNull();
    expect(payload.project_id).toBe(fixtureProject.id);
  });

  it('"new chat in project" (Sidebar) emits project_selected with the new session id, after the session exists', async () => {
    const user = userEvent.setup();
    render(<AppShell />);

    await user.click(
      screen.getByRole("button", { name: "new-chat-in-project" }),
    );

    await waitFor(() => expect(emitCalls().length).toBeGreaterThan(0));

    const [, args] = emitCalls()[emitCalls().length - 1];
    const payload = args as Record<string, unknown>;
    expect(payload.event_type).toBe("project_selected");
    expect(payload.session_id).toEqual(expect.stringMatching(/^acp-session-\d+$/));
    expect(payload.session_id).not.toBeNull();
    expect(payload.project_id).toBe(fixtureProject.id);
  });
});
