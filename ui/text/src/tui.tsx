#!/usr/bin/env node
import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { Box, Text, render, useApp, useInput, useStdout } from "ink";
import { MultilineInput } from "ink-multiline-input";
import meow from "meow";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { Readable, Writable } from "node:stream";
import type {
  SessionNotification,
  RequestPermissionRequest,
  RequestPermissionResponse,
  Stream,
  ContentChunk,
  ToolCall,
  ToolCallUpdate,
} from "@agentclientprotocol/sdk";
import { ndJsonStream } from "@agentclientprotocol/sdk";
import { GooseClient } from "@aaif/goose-sdk";
import { resolveGooseBinary } from "@aaif/goose-sdk/node";
import Onboarding from "./onboarding.js";
import ConfigureScreen, { ConfigureIntent } from "./configure.js";
import ExtensionsManager from "./extensions.js";
import type { PendingPermission, Turn, QueuedMessage } from "./types.js";
import {
  emptyLine,
  renderUserPrompt,
  renderToolCallItem,
  renderErrorItem,
  renderContentItem,
  renderLoadingIndicator,
  renderQueuedMessages,
} from "./components/ContentRenderers.js";
import { Header } from "./components/Header.js";
import { Footer } from "./components/Footer.js";
import { Rule } from "./components/Rule.js";
import { SlashMenu, slashMenuHeight } from "./components/SlashMenu.js";
import { FileMenu, pickListHeight } from "./components/FileMenu.js";
import { InfoOverlay } from "./components/InfoOverlay.js";
import { isErrorStatus, formatError } from "./utils.js";
import { formatHomePath, projectFolderName } from "./projectPath.js";
import {
  filterSlashCommands,
  formatHelpText,
  gitDiffSummary,
  isSlashMenuOpen,
  parseSlashInput,
  resolveSlashCommand,
  resolveUserPath,
  slashStem,
  type SlashCommandDef,
} from "./slashCommands.js";
import {
  CRANBERRY,
  TEAL,
  GOLD,
  TEXT_PRIMARY,
  TEXT_SECONDARY,
  TEXT_DIM,
  RULE_COLOR,
} from "./colors.js";
import {
  applyAtMention,
  atQuery,
  copyToClipboard,
  defaultExportPath,
  formatShellPrompt,
  formatTranscript,
  fuzzyFiles,
  harnessFacts,
  lastAssistantText,
  listProjectFiles,
  parseBang,
  runShellCommand,
  stripAtQuery,
  writeTranscript,
} from "./editorExtras.js";
import {
  CONTINUE_PROMPT,
  enableAutonomous,
  formatAutonomousStatus,
  idleAutonomous,
  parseAutonomousArgs,
  runGateCommand,
  shouldAutoContinue,
  type AutonomousState,
} from "./autonomous.js";
import {
  estimateTokensFromChars,
  formatModeHelp,
  formatTokenCount,
  MODE_LABEL,
  nextSessionMode,
  parseSessionMode,
  resolveAcpModeId,
  type SessionMode,
} from "./sessionMode.js";
import {
  MOBIUS_H,
  MOBIUS_INTRO_FRAMES,
  MOBIUS_INTERVAL_MS,
  getMobiusIntroFrame,
} from "./mobius.js";
import { Spinner, SPINNER_FRAMES } from "./components/Spinner.js";
import {
  PASTE_THRESHOLD,
  INPUT_MAX_ROWS,
  SENT_PREVIEW_LEN,
  INITIAL_GREETING,
  PERMISSION_LABELS,
  PERMISSION_KEYS,
} from "./constants.js";

function composerHint({
  busy,
  queuedKind,
  isPasteMode,
  slashOpen,
  atOpen,
  autonomous,
}: {
  busy: boolean;
  queuedKind: "steer" | "followup" | null;
  isPasteMode: boolean;
  slashOpen: boolean;
  atOpen: boolean;
  autonomous: boolean;
}): { text: string; color: string } {
  if (isPasteMode) {
    return { text: "enter to send · esc to clear", color: TEXT_DIM };
  }
  if (slashOpen) {
    return {
      text: "enter to run · tab to complete · ↑↓ · esc to dismiss",
      color: GOLD,
    };
  }
  if (atOpen) {
    return {
      text: "enter insert file · tab complete · ↑↓ · esc to dismiss",
      color: GOLD,
    };
  }
  if (queuedKind === "steer") {
    return {
      text: "steer queued — cuts in when this turn stops",
      color: GOLD,
    };
  }
  if (queuedKind === "followup") {
    return {
      text: "follow-up queued · enter steers now · esc drops queue",
      color: GOLD,
    };
  }
  if (busy) {
    return {
      text: autonomous
        ? "esc hard-stop · enter steer · alt+enter queue · auto on"
        : "esc hard-stop · enter steer · alt+enter queue",
      color: TEXT_DIM,
    };
  }
  return {
    text: "enter send · shift+tab mode · @ files · ! shell · /help",
    color: TEXT_DIM,
  };
}

const InputBar = React.memo(function InputBar({
  width,
  input,
  onChange,
  onSubmit,
  queued,
  busy,
  scrollHint,
  placeholder,
  focused,
  pastedFull,
  onPastedFullChange,
  slashOpen,
  atOpen,
  autonomous,
  queuedKind,
  onFollowUp,
}: {
  width: number;
  input: string;
  onChange: (v: string) => void;
  onSubmit: (v: string) => void;
  queued: boolean;
  busy: boolean;
  scrollHint: boolean;
  placeholder?: string;
  focused: boolean;
  pastedFull: string | null;
  onPastedFullChange: (v: string | null) => void;
  slashOpen: boolean;
  atOpen: boolean;
  autonomous: boolean;
  queuedKind: "steer" | "followup" | null;
  onFollowUp: (v: string) => void;
}) {
  const prevLenRef = useRef(input.length);

  const handleChange = useCallback(
    (newValue: string) => {
      const delta = newValue.length - prevLenRef.current;
      prevLenRef.current = newValue.length;
      if (delta >= PASTE_THRESHOLD) {
        onPastedFullChange(newValue);
        onChange(newValue);
      } else {
        if (pastedFull !== null) onPastedFullChange(null);
        onChange(newValue);
      }
    },
    [onChange, pastedFull, onPastedFullChange],
  );

  const handleSubmit = useCallback(
    (value: string) => {
      prevLenRef.current = 0;
      onPastedFullChange(null);
      onSubmit(value);
    },
    [onSubmit, onPastedFullChange],
  );

  const handleFollowUp = useCallback(
    (value: string) => {
      prevLenRef.current = 0;
      onPastedFullChange(null);
      onFollowUp(value);
    },
    [onFollowUp, onPastedFullChange],
  );

  useInput(
    (ch, key) => {
      if (key.return) {
        if (key.alt) handleFollowUp(input);
        else handleSubmit(input);
        return;
      }
      if (key.backspace || key.delete) {
        prevLenRef.current = 0;
        onPastedFullChange(null);
        onChange("");
        return;
      }
      if (key.escape) {
        prevLenRef.current = 0;
        onPastedFullChange(null);
        onChange("");
        return;
      }
      if (ch && !key.ctrl && !key.meta) {
        prevLenRef.current = ch.length;
        onPastedFullChange(null);
        onChange(ch);
      }
    },
    { isActive: focused && pastedFull !== null },
  );

  const isPasteMode = pastedFull !== null;
  const constrainedWidth = Math.max(width, 20);
  const contentWidth = Math.max(constrainedWidth - 6, 10);
  const hint = composerHint({
    busy,
    queuedKind,
    isPasteMode,
    slashOpen,
    atOpen,
    autonomous,
  });
  const borderColor = busy || queued ? GOLD : TEAL;
  const promptColor = busy ? GOLD : CRANBERRY;
  const idlePlaceholder = placeholder ?? "Message the agent";

  return (
    <Box
      flexDirection="column"
      borderStyle="round"
      borderColor={borderColor}
      paddingX={1}
      width={constrainedWidth}
      flexShrink={0}
    >
      <Box>
        <Text color={promptColor} bold>
          {"❯ "}
        </Text>
        {isPasteMode ? (
          <Box width={contentWidth} justifyContent="space-between">
            <Box width={Math.max(contentWidth - 20, 10)}>
              <Text color={TEXT_PRIMARY} wrap="truncate-end">
                {(() => {
                  const text = pastedFull;
                  const availableWidth = Math.max(contentWidth - 20, 10);
                  const flat = text
                    .replace(/\n/g, " ")
                    .replace(/\s+/g, " ")
                    .trim();
                  if (flat.length <= availableWidth) return flat;
                  const suffix = ` (${flat.length.toLocaleString()} chars)`;
                  const previewLen = Math.max(
                    availableWidth - suffix.length - 1,
                    5,
                  );
                  return flat.slice(0, previewLen) + "…" + suffix;
                })()}
              </Text>
            </Box>
            {scrollHint && <Text color={TEXT_DIM}>shift+↑↓ history</Text>}
          </Box>
        ) : (
          <Box flexGrow={1} justifyContent="space-between">
            <MultilineInput
              value={input}
              onChange={handleChange}
              onSubmit={handleSubmit}
              rows={1}
              maxRows={INPUT_MAX_ROWS}
              placeholder={busy ? "Steer or queue a follow-up…" : idlePlaceholder}
              focus={focused}
              keyBindings={{
                submit: (key) =>
                  key.return && !key.ctrl && !key.alt && !key.shift,
                newline: (key) =>
                  (key.return && key.ctrl) || (key.return && key.shift),
              }}
              useCustomInput={(handler, isActive) => {
                useInput(
                  (ch, key) => {
                    if (key.shift && (key.upArrow || key.downArrow)) return;
                    if (key.alt && (key.upArrow || key.downArrow)) return;
                    if (key.tab && (key.shift || key.meta || key.ctrl)) return;
                    if (key.return && key.alt && !key.ctrl) {
                      handleFollowUp(input);
                      return;
                    }
                    if (
                      (slashOpen || atOpen) &&
                      (key.upArrow || key.downArrow || key.tab || key.return)
                    ) {
                      return;
                    }
                    handler(ch, key);
                  },
                  { isActive },
                );
              }}
            />
            {scrollHint && <Text color={TEXT_DIM}>shift+↑↓ history</Text>}
          </Box>
        )}
      </Box>
      <Box>
        <Text color={hint.color} italic wrap="truncate-end">
          {hint.text}
        </Text>
      </Box>
    </Box>
  );
});

function buildContentLines({
  turn,
  turnIndex,
  width,
  loading,
  status,
  spinIdx,
  pendingPermission,
  permissionIdx,
  toolCallsExpanded,
  queuedMessages,
}: {
  turn: Turn | undefined;
  turnIndex: number;
  width: number;
  loading: boolean;
  status: string;
  spinIdx: number;
  pendingPermission: PendingPermission | null;
  permissionIdx: number;
  toolCallsExpanded: boolean;
  queuedMessages: QueuedMessage[];
}): React.ReactElement[] {
  const lines: React.ReactElement[] = [];
  if (!turn) return lines;

  const safeWidth = Math.max(width, 20);

  const turnId = String(turnIndex);
  lines.push(
    ...renderUserPrompt(
      turn.userText,
      safeWidth,
      turnId,
      (text: string, availableWidth: number) => {
        const flat = text.replace(/\n/g, " ").replace(/\s+/g, " ").trim();
        const safeWidth = Math.max(availableWidth, 10);
        const maxPreview = Math.max(
          safeWidth - 30,
          Math.min(SENT_PREVIEW_LEN, safeWidth - 10),
        );
        if (flat.length <= maxPreview + 10) {
          return (
            <Box width={safeWidth}>
              <Text color={TEXT_PRIMARY} bold wrap="wrap">
                {flat}
              </Text>
            </Box>
          );
        }
        const preview = flat.slice(0, maxPreview) + "…";
        const remaining = flat.length - maxPreview;
        return (
          <Box width={safeWidth}>
            <Text color={TEXT_PRIMARY} bold wrap="wrap">
              {preview}
            </Text>
            <Text color={TEXT_DIM}>
              {" "}
              ({remaining.toLocaleString()} more chars)
            </Text>
          </Box>
        );
      },
    ),
  );

  // Process response items
  const hasToolCalls = turn.responseItems.some(
    (it) => it.itemType === "tool_call",
  );
  let tcIdx = 0;

  for (let i = 0; i < turn.responseItems.length; i++) {
    const item = turn.responseItems[i]!;

    if (item.itemType === "tool_call") {
      lines.push(
        ...renderToolCallItem(
          item,
          i,
          safeWidth,
          toolCallsExpanded,
          tcIdx === 0,
          hasToolCalls,
        ),
      );
      tcIdx++;
    } else if (item.itemType === "error") {
      lines.push(...renderErrorItem(item, i, safeWidth));
    } else if (item.itemType === "content_chunk") {
      lines.push(...renderContentItem(item, i, safeWidth));
    }
  }

  // Loading indicator
  if (loading && !pendingPermission) {
    lines.push(...renderLoadingIndicator(status, spinIdx, safeWidth));
  }

  // Permission dialog
  if (pendingPermission) {
    const perm = pendingPermission;
    const selectedIdx = permissionIdx;
    const fullWidth = safeWidth;
    const dialogWidth = Math.min(fullWidth - 2, 58);
    const innerWidth = Math.max(dialogWidth - 4, 10);
    const hRule = "─".repeat(Math.max(dialogWidth - 2, 0));
    const permissionLines: React.ReactElement[] = [];

    permissionLines.push(
      emptyLine(
        `pm-gap-${perm.toolTitle.slice(0, 10).replace(/[^a-zA-Z0-9]/g, "")}`,
        fullWidth,
      ),
    );

    permissionLines.push(
      <Box key="pm-t" width={fullWidth} height={1}>
        <Text color={GOLD}>╭{hRule}╮</Text>
      </Box>,
    );

    const row = (key: string, content: React.ReactNode) => {
      permissionLines.push(
        <Box key={key} width={fullWidth} height={1}>
          <Text color={GOLD}>│ </Text>
          <Box width={innerWidth} height={1}>
            {content}
          </Box>
          <Text color={GOLD}> │</Text>
        </Box>,
      );
    };

    row(
      "pm-title",
      <Text color={GOLD} bold>
        🔒 Permission required
      </Text>,
    );
    row("pm-g1", <Text> </Text>);
    row(
      "pm-tool",
      <Text wrap="truncate-end" color={TEXT_PRIMARY}>
        {perm.toolTitle}
      </Text>,
    );
    row("pm-g2", <Text> </Text>);

    for (let i = 0; i < perm.options.length; i++) {
      const opt = perm.options[i]!;
      const k = PERMISSION_KEYS[opt.kind] ?? String(i + 1);
      const label = PERMISSION_LABELS[opt.kind] ?? opt.name;
      const active = i === selectedIdx;
      row(
        `pm-o${i}`,
        <>
          <Text color={active ? GOLD : RULE_COLOR}>{active ? "▸ " : "  "}</Text>
          <Text color={active ? TEXT_PRIMARY : TEXT_SECONDARY} bold={active}>
            [{k}] {label}
          </Text>
        </>,
      );
    }

    row("pm-g3", <Text> </Text>);
    row(
      "pm-help",
      <Text color={TEXT_DIM}>↑↓ select · enter confirm · esc cancel</Text>,
    );

    permissionLines.push(
      <Box key="pm-b" width={fullWidth} height={1}>
        <Text color={GOLD}>╰{hRule}╯</Text>
      </Box>,
    );

    lines.push(...permissionLines);
  }

  // Queued messages
  lines.push(...renderQueuedMessages(queuedMessages, safeWidth));

  return lines;
}

const Viewport = React.memo(function Viewport({
  lines,
  height,
  width,
  scrollOffset,
}: {
  lines: React.ReactElement[];
  height: number;
  width: number;
  scrollOffset: number;
}) {
  const total = lines.length;
  const overflows = total > height;

  const contentHeight = overflows ? Math.max(height - 2, 1) : height;

  const maxEnd = total;
  const minEnd = Math.min(contentHeight, total);
  const endIdx = Math.max(minEnd, Math.min(maxEnd - scrollOffset, maxEnd));
  const startIdx = Math.max(0, endIdx - contentHeight);

  const visible = lines.slice(startIdx, endIdx);

  const padCount = contentHeight - visible.length;

  const elements: React.ReactElement[] = [];

  if (overflows) {
    const above = startIdx;
    elements.push(
      <Box key="si-up" width={width} height={1} justifyContent="center">
        {above > 0 ? (
          <Text color={TEXT_DIM}>▲ {above} more (↑)</Text>
        ) : (
          <Text> </Text>
        )}
      </Box>,
    );
  }

  for (let i = 0; i < padCount; i++) {
    elements.push(emptyLine(`vp-pad-${i}`, width));
  }
  elements.push(...visible);

  if (overflows) {
    const below = total - endIdx;
    elements.push(
      <Box key="si-dn" width={width} height={1} justifyContent="center">
        {below > 0 ? (
          <Text color={TEXT_DIM}>▼ {below} more (↓)</Text>
        ) : (
          <Text> </Text>
        )}
      </Box>,
    );
  }

  const constrainedWidth = Math.max(width, 10);
  const constrainedHeight = Math.max(height, 1);

  return (
    <Box
      flexDirection="column"
      height={constrainedHeight}
      width={constrainedWidth}
    >
      {elements}
    </Box>
  );
});

const SplashScreen = React.memo(function SplashScreen({
  width,
  height,
  status,
  loading,
  spinIdx,
  projectName,
  projectPath,
  contextLine,
  mobiusFrame,
}: {
  width: number;
  height: number;
  status: string;
  loading: boolean;
  spinIdx: number;
  projectName: string;
  projectPath: string;
  contextLine: string;
  mobiusFrame: number;
}) {
  const statusColor =
    status === "ready" ? TEAL : isErrorStatus(status) ? CRANBERRY : TEXT_DIM;
  const ribbon = getMobiusIntroFrame(mobiusFrame);

  const contentHeight = MOBIUS_H + 1 + 1 + 1 + 1 + 2 + 1;
  const topPad = Math.max(0, Math.floor((height - contentHeight) / 2));
  const safeWidth = Math.max(width, 20);
  const safeHeight = Math.max(height, 10);

  return (
    <Box
      flexDirection="column"
      alignItems="center"
      width={safeWidth}
      height={safeHeight}
      overflow="hidden"
    >
      {topPad > 0 && <Box height={topPad} />}
      <Box flexDirection="column" alignItems="center">
        {ribbon.map((runs, i) => (
          <Text key={i}>
            {runs.map((run, j) => (
              <Text key={j} color={run.color}>
                {run.text}
              </Text>
            ))}
          </Text>
        ))}
      </Box>
      <Box marginTop={1}>
        <Text color={TEXT_PRIMARY} bold>
          permagent
        </Text>
        <Text color={RULE_COLOR}> · </Text>
        <Text color={TEXT_PRIMARY}>{projectName}</Text>
      </Box>
      <Box alignItems="center">
        <Text color={TEXT_DIM} wrap="truncate-start">
          {projectPath}
        </Text>
      </Box>
      <Box alignItems="center">
        <Text color={TEXT_DIM} wrap="truncate-end">
          {contextLine}
        </Text>
      </Box>
      <Box marginTop={2} gap={1} alignItems="center">
        {loading && <Spinner idx={spinIdx} />}
        <Text color={statusColor}>{status}</Text>
      </Box>
    </Box>
  );
});

function App({
  serverConnection,
  initialPrompt,
}: {
  serverConnection: Stream | string;
  initialPrompt?: string;
}) {
  const { exit } = useApp();
  const { stdout } = useStdout();
  const termWidth = stdout?.columns ?? 80;
  const termHeight = stdout?.rows ?? 24;

  const [turns, setTurns] = useState<Turn[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(true);
  const [status, setStatus] = useState("connecting…");
  const [spinIdx, setSpinIdx] = useState(0);
  const [mobiusFrame, setMobiusFrame] = useState(0);
  const [splashGen, setSplashGen] = useState(0);
  const [bannerVisible, setBannerVisible] = useState(true);
  const [pendingPermission, setPendingPermission] =
    useState<PendingPermission | null>(null);
  const [permissionIdx, setPermissionIdx] = useState(0);
  const [queuedMessages, setQueuedMessages] = useState<QueuedMessage[]>([]);
  const [sessionMode, setSessionMode] = useState<SessionMode>("auto");
  const [modelName, setModelName] = useState("");
  const [autonomous, setAutonomous] = useState<AutonomousState>(idleAutonomous);

  const [viewTurnIdx, setViewTurnIdx] = useState(-1);
  const [toolCallsExpanded, setToolCallsExpanded] = useState(false);
  const [scrollOffset, setScrollOffset] = useState(0);
  const [pastedFull, setPastedFull] = useState<string | null>(null);
  const [needsOnboarding, setNeedsOnboarding] = useState(false);
  type Overlay =
    | { screen: "configure"; intent: ConfigureIntent }
    | { screen: "extensions" }
    | { screen: "info"; title: string; body: string };
  const [overlay, setOverlay] = useState<Overlay | null>(null);
  const [slashIdx, setSlashIdx] = useState(0);
  const [atIdx, setAtIdx] = useState(0);
  const [projectCwd, setProjectCwd] = useState(() => process.cwd());
  const [projectFiles, setProjectFiles] = useState<string[]>([]);

  const clientRef = useRef<GooseClient | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const streamBuf = useRef("");
  const sentInitialPrompt = useRef(false);
  const queueRef = useRef<QueuedMessage[]>([]);
  const isProcessingRef = useRef(false);
  const loadingRef = useRef(loading);
  loadingRef.current = loading;
  const cancelRequestedRef = useRef(false);
  const hardStopRef = useRef(false);
  const autonomousRef = useRef(autonomous);
  autonomousRef.current = autonomous;
  const sessionModeRef = useRef(sessionMode);
  sessionModeRef.current = sessionMode;
  const availableModesRef = useRef<Array<{ id: string }>>([]);
  const projectCwdRef = useRef(projectCwd);
  projectCwdRef.current = projectCwd;

  const projectName = projectFolderName(projectCwd);
  const projectPath = formatHomePath(projectCwd);
  const contextLine = useMemo(() => {
    const facts = harnessFacts(projectCwd);
    return facts.length
      ? facts.join(" · ")
      : "shift+tab mode · @ files · ! shell · /help";
  }, [projectCwd]);

  useEffect(() => {
    let cancelled = false;
    void listProjectFiles(projectCwd).then((files) => {
      if (!cancelled) setProjectFiles(files);
    });
    return () => {
      cancelled = true;
    };
  }, [projectCwd]);

  // Möbius intro: same comet sweep the CLI harness plays on launch (~1.2s),
  // then the ribbon holds until the first message hides the splash.
  useEffect(() => {
    if (!bannerVisible) return;
    setMobiusFrame(0);
    const last = MOBIUS_INTRO_FRAMES - 1;
    let f = 0;
    const t = setInterval(() => {
      f += 1;
      setMobiusFrame(Math.min(f, last));
      if (f >= last) clearInterval(t);
    }, MOBIUS_INTERVAL_MS);
    return () => clearInterval(t);
  }, [bannerVisible, splashGen]);

  // Only tick the spinner while a turn is in flight.
  useEffect(() => {
    if (!loading) return;
    const t = setInterval(() => {
      setSpinIdx((i) => (i + 1) % SPINNER_FRAMES.length);
    }, 300);
    return () => clearInterval(t);
  }, [loading]);

  useEffect(() => {
    if (turns.length > 0) setBannerVisible(false);
  }, [turns]);

  useEffect(() => {
    setToolCallsExpanded(false);
    setScrollOffset(0);
  }, [viewTurnIdx, turns.length]);

  const appendAgent = useCallback((text: string) => {
    setTurns((prev) => {
      if (prev.length === 0) return prev;
      const last = { ...prev[prev.length - 1]! };
      const newItems = [...last.responseItems];

      if (
        newItems.length > 0 &&
        newItems[newItems.length - 1]!.itemType === "content_chunk"
      ) {
        const lastItem = newItems[newItems.length - 1] as ContentChunk & {
          itemType: "content_chunk";
        };
        if (lastItem.content.type === "text") {
          newItems[newItems.length - 1] = {
            ...lastItem,
            content: {
              ...lastItem.content,
              text: lastItem.content.text + text,
            },
          };
        } else {
          newItems.push({
            itemType: "content_chunk",
            content: { type: "text", text },
          });
        }
      } else {
        newItems.push({
          itemType: "content_chunk",
          content: { type: "text", text },
        });
      }

      return [...prev.slice(0, -1), { ...last, responseItems: newItems }];
    });
  }, []);

  const appendError = useCallback((errorMessage: string) => {
    setTurns((prev) => {
      if (prev.length === 0) return prev;
      const last = { ...prev[prev.length - 1]! };
      const newItems = [...last.responseItems];
      newItems.push({ itemType: "error", message: errorMessage });
      return [...prev.slice(0, -1), { ...last, responseItems: newItems }];
    });
  }, []);

  const handleToolCall = useCallback((tc: ToolCall) => {
    setTurns((prev) => {
      if (prev.length === 0) return prev;
      const last = { ...prev[prev.length - 1]! };
      const newItems = [...last.responseItems];
      const newById = new Map(last.toolCallsById);
      const index = newItems.length;
      newItems.push({ ...tc, itemType: "tool_call" });
      newById.set(tc.toolCallId, index);
      return [
        ...prev.slice(0, -1),
        { ...last, responseItems: newItems, toolCallsById: newById },
      ];
    });
  }, []);

  const handleToolCallUpdate = useCallback((update: ToolCallUpdate) => {
    setTurns((prev) => {
      if (prev.length === 0) return prev;
      const last = { ...prev[prev.length - 1]! };
      const index = last.toolCallsById.get(update.toolCallId);
      if (index === undefined) return prev;
      const item = last.responseItems[index];
      if (!item || item.itemType !== "tool_call") return prev;
      const updated: ToolCall & { itemType: "tool_call" } = { ...item };
      if (update.title != null) updated.title = update.title;
      if (update.status != null) updated.status = update.status;
      if (update.kind != null) updated.kind = update.kind;
      if (update.rawInput !== undefined) updated.rawInput = update.rawInput;
      if (update.rawOutput !== undefined) updated.rawOutput = update.rawOutput;
      if (update.content != null) updated.content = update.content;
      if (update.locations != null) updated.locations = update.locations;
      const newItems = [...last.responseItems];
      newItems[index] = updated;
      return [...prev.slice(0, -1), { ...last, responseItems: newItems }];
    });
  }, []);

  const addUserTurn = useCallback((text: string) => {
    setTurns((prev) => [
      ...prev,
      { userText: text, responseItems: [], toolCallsById: new Map() },
    ]);
    setViewTurnIdx(-1);
    setToolCallsExpanded(false);
    setScrollOffset(0);
  }, []);

  const resolvePermission = useCallback(
    (option: { optionId: string } | "cancelled") => {
      if (!pendingPermission) return;
      const { resolve } = pendingPermission;
      if (option === "cancelled") {
        resolve({ outcome: { outcome: "cancelled" } });
      } else {
        resolve({
          outcome: { outcome: "selected", optionId: option.optionId },
        });
      }
      setPendingPermission(null);
      setPermissionIdx(0);
    },
    [pendingPermission],
  );

  const executePrompt = useCallback(
    async (text: string): Promise<{ reason: string; cancelled: boolean }> => {
      const client = clientRef.current;
      const sid = sessionIdRef.current;
      if (!client || !sid) return { reason: "error", cancelled: false };

      addUserTurn(text);
      setLoading(true);
      setStatus("thinking…");
      streamBuf.current = "";

      try {
        const result = await client.prompt({
          sessionId: sid,
          prompt: [{ type: "text", text }],
        });
        if (streamBuf.current) appendAgent("");
        if (result.stopReason === "end_turn") {
          setStatus("ready");
          return { reason: "end_turn", cancelled: false };
        }
        if (result.stopReason === "cancelled") {
          setStatus("stopped");
          return { reason: "cancelled", cancelled: true };
        }
        setStatus(`stopped: ${result.stopReason}`);
        return { reason: result.stopReason, cancelled: false };
      } catch (e: unknown) {
        if (cancelRequestedRef.current) {
          setStatus("stopped");
          return { reason: "cancelled", cancelled: true };
        }
        const errorMsg = formatError(e);
        setStatus(`error`);
        appendError(brandCopy(errorMsg));
        return { reason: "error", cancelled: false };
      } finally {
        cancelRequestedRef.current = false;
        setLoading(false);
      }
    },
    [appendAgent, appendError, addUserTurn],
  );

  const takeNextQueued = useCallback((): QueuedMessage | undefined => {
    const list = queueRef.current;
    if (list.length === 0) return undefined;
    const steerIdx = list.findIndex((q) => q.kind === "steer");
    const idx = steerIdx >= 0 ? steerIdx : 0;
    const [item] = list.splice(idx, 1);
    setQueuedMessages([...list]);
    return item;
  }, []);

  const pump = useCallback(
    async (last: { reason: string; cancelled: boolean }) => {
      if (isProcessingRef.current) return;
      isProcessingRef.current = true;
      try {
        let current = last;
        while (true) {
          if (hardStopRef.current) {
            hardStopRef.current = false;
            break;
          }
          const next = takeNextQueued();
          if (next) {
            current = await executePrompt(next.text);
            continue;
          }
          const auto = autonomousRef.current;
          const decision = shouldAutoContinue(auto, {
            stopReason: current.reason,
            queueEmpty: true,
            cancelled: current.cancelled,
          });
          if (!decision.continue) {
            if (decision.reason && auto.enabled) {
              setAutonomous((s) => ({ ...s, enabled: false }));
              setStatus(decision.reason);
            }
            break;
          }
          if (auto.gate) {
            setStatus("gate…");
            const gate = await runGateCommand(auto.gate, projectCwdRef.current);
            if (!gate.ok) {
              setAutonomous((s) => ({ ...s, turnsUsed: s.turnsUsed + 1 }));
              current = await executePrompt(
                `Quality gate failed:\n${gate.output}\nFix this and continue.`,
              );
              continue;
            }
          }
          setAutonomous((s) => ({ ...s, turnsUsed: s.turnsUsed + 1 }));
          current = await executePrompt(CONTINUE_PROMPT);
        }
      } finally {
        isProcessingRef.current = false;
      }
    },
    [executePrompt, takeNextQueued],
  );

  const sendPrompt = useCallback(
    async (text: string) => {
      const result = await executePrompt(text);
      await pump(result);
    },
    [executePrompt, pump],
  );

  const enqueue = useCallback((item: QueuedMessage) => {
    queueRef.current.push(item);
    setQueuedMessages([...queueRef.current]);
  }, []);

  const drainQueue = useCallback(() => {
    queueRef.current = [];
    setQueuedMessages([]);
  }, []);

  const interruptTurn = useCallback(
    async (opts?: { drain?: boolean }) => {
      const client = clientRef.current;
      const sid = sessionIdRef.current;
      if (opts?.drain) {
        hardStopRef.current = true;
        drainQueue();
        setAutonomous((s) =>
          s.enabled ? { ...s, enabled: false, turnsUsed: 0 } : s,
        );
      }
      if (!client || !sid) return;
      if (!loadingRef.current && !isProcessingRef.current) {
        if (opts?.drain) setStatus("stopped");
        return;
      }
      if (cancelRequestedRef.current) return;
      cancelRequestedRef.current = true;
      setStatus(opts?.drain ? "stopped" : "steering…");
      try {
        await client.cancel({ sessionId: sid });
      } catch {
        // prompt() will settle with cancelled / error
      }
    },
    [drainQueue],
  );

  const createSession = useCallback(
    async (client: GooseClient) => {
      setStatus("creating session…");
      setLoading(true);
      try {
        const session = await client.newSession({
          cwd: projectCwd,
          mcpServers: [],
        });
        sessionIdRef.current = session.sessionId;
        const sess = session as typeof session & {
          modes?: {
            currentModeId?: string;
            availableModes?: Array<{ id: string }>;
          };
        };
        if (sess.modes?.availableModes) {
          availableModesRef.current = sess.modes.availableModes;
        }
        try {
          const modeRaw = await client.goose.GooseConfigRead({
            key: "GOOSE_MODE",
          });
          setSessionMode(
            parseSessionMode(String(modeRaw.value ?? "auto")) ?? "auto",
          );
        } catch {
          setSessionMode("auto");
        }
        try {
          const modelRaw = await client.goose.GooseConfigRead({
            key: "GOOSE_MODEL",
          });
          if (typeof modelRaw.value === "string") setModelName(modelRaw.value);
        } catch {
          // footer still works without a model name
        }
        setLoading(false);
        setStatus("ready");

        if (initialPrompt && !sentInitialPrompt.current) {
          sentInitialPrompt.current = true;
          await sendPrompt(initialPrompt);
          setTimeout(() => exit(), 100);
        }
      } catch (e: unknown) {
        const errorMsg = formatError(e);
        setStatus(`failed: ${errorMsg}`);
        setLoading(false);
      }
    },
    [initialPrompt, sendPrompt, exit, projectCwd],
  );

  const handleOnboardingComplete = useCallback(() => {
    setNeedsOnboarding(false);
    const client = clientRef.current;
    if (client) createSession(client);
  }, [createSession]);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        setStatus("initializing…");

        const client = new GooseClient(
          () => ({
            sessionUpdate: async (params: SessionNotification) => {
              const update = params.update;
              if (update.sessionUpdate === "agent_message_chunk") {
                if (update.content.type === "text") {
                  streamBuf.current += update.content.text;
                  appendAgent(update.content.text);
                }
              } else if (update.sessionUpdate === "tool_call") {
                handleToolCall(update);
              } else if (update.sessionUpdate === "tool_call_update") {
                handleToolCallUpdate(update);
              }
            },
            requestPermission: async (
              params: RequestPermissionRequest,
            ): Promise<RequestPermissionResponse> => {
              return new Promise<RequestPermissionResponse>((resolve) => {
                setPendingPermission({
                  toolTitle: params.toolCall.title ?? "unknown tool",
                  options: params.options.map((o) => ({
                    optionId: o.optionId,
                    name: o.name,
                    kind: o.kind,
                  })),
                  resolve,
                });
                setPermissionIdx(0);
              });
            },
          }),
          serverConnection,
        );

        if (cancelled) return;
        clientRef.current = client;

        setStatus("handshaking…");
        await client.initialize({
          protocolVersion: 0,
          clientInfo: { name: "permagent-tui", version: "0.1.0" },
          clientCapabilities: {},
        });
        if (cancelled) return;

        setStatus("checking provider…");
        let hasProvider = false;
        try {
          const resp = await client.goose.GooseConfigRead({
            key: "GOOSE_PROVIDER",
          });
          hasProvider =
            resp.value != null && resp.value !== "" && resp.value !== "null";
        } catch {
          hasProvider = false;
        }
        if (cancelled) return;

        if (!hasProvider && !initialPrompt) {
          setNeedsOnboarding(true);
          setLoading(false);
          setStatus("setup required");
          return;
        }

        await createSession(client);
      } catch (e: unknown) {
        if (cancelled) return;
        const errorMsg = formatError(e);
        setStatus(`failed: ${errorMsg}`);
        setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    serverConnection,
    initialPrompt,
    createSession,
    appendAgent,
    handleToolCall,
    handleToolCallUpdate,
    exit,
  ]);

  const slashOpen = isSlashMenuOpen(input);
  const slashMatches = useMemo(
    () => (slashOpen ? filterSlashCommands(slashStem(input)) : []),
    [slashOpen, input],
  );
  const at = useMemo(() => atQuery(input), [input]);
  const atOpen = at !== null;
  const atMatches = useMemo(
    () => (at ? fuzzyFiles(projectFiles, at.query) : []),
    [at, projectFiles],
  );

  useEffect(() => {
    setSlashIdx(0);
    setAtIdx(0);
  }, [input]);

  const showInfo = useCallback((title: string, body: string) => {
    setOverlay({ screen: "info", title, body });
  }, []);

  const applyModelName = useCallback(
    async (model: string) => {
      const client = clientRef.current;
      const sid = sessionIdRef.current;
      if (!client || !sid) return;
      const providerRaw = await client.goose.GooseConfigRead({
        key: "GOOSE_PROVIDER",
      });
      const provider =
        typeof providerRaw.value === "string" ? providerRaw.value : "";
      await client.goose.GooseConfigUpsert({ key: "GOOSE_MODEL", value: model });
      const ext = client.goose as GooseClient["goose"] & {
        GooseSessionProviderUpdate?: (p: {
          sessionId: string;
          provider: string;
          model: string;
        }) => Promise<void>;
      };
      if (provider && typeof ext.GooseSessionProviderUpdate === "function") {
        await ext.GooseSessionProviderUpdate({
          sessionId: sid,
          provider,
          model,
        });
      }
      try {
        await client.unstable_setSessionModel({ sessionId: sid, modelId: model });
      } catch {
        // session model ACP method is optional
      }
      setStatus(`model → ${model}`);
      setModelName(model);
    },
    [],
  );

  const applySessionMode = useCallback(async (mode: SessionMode) => {
    setSessionMode(mode);
    const client = clientRef.current;
    const sid = sessionIdRef.current;
    if (!client) return;
    await client.goose.GooseConfigUpsert({ key: "GOOSE_MODE", value: mode });
    if (sid) {
      try {
        await client.setSessionMode({
          sessionId: sid,
          modeId: resolveAcpModeId(mode, availableModesRef.current),
        });
      } catch {
        // ACP session modes are optional; config still applies to new turns
      }
    }
    setStatus(`mode → ${MODE_LABEL[mode]}`);
  }, []);

  const cycleSessionMode = useCallback(() => {
    void applySessionMode(nextSessionMode(sessionModeRef.current));
  }, [applySessionMode]);

  const runSlash = useCallback(
    async (cmd: SlashCommandDef, args: string) => {
      setInput("");
      setPastedFull(null);
      switch (cmd.name) {
        case "model":
          if (args) {
            try {
              await applyModelName(args);
            } catch (e: unknown) {
              setStatus(`failed: ${formatError(e)}`);
            }
            return;
          }
          setOverlay({ screen: "configure", intent: "model" });
          return;
        case "mode": {
          if (!args) {
            showInfo("mode", formatModeHelp(sessionModeRef.current));
            return;
          }
          const parsed = parseSessionMode(args);
          if (!parsed) {
            setStatus(`unknown mode: ${args}  —  auto | ask | chat`);
            return;
          }
          try {
            await applySessionMode(parsed);
          } catch (e: unknown) {
            setStatus(`failed: ${formatError(e)}`);
          }
          return;
        }
        case "autonomous": {
          const parsed = parseAutonomousArgs(args);
          if (parsed.action === "error") {
            showInfo("autonomous", parsed.message);
            return;
          }
          if (parsed.action === "status") {
            showInfo("autonomous", formatAutonomousStatus(autonomousRef.current));
            return;
          }
          if (parsed.action === "off") {
            setAutonomous((s) => ({ ...s, enabled: false }));
            setStatus("autonomous off");
            return;
          }
          if (parsed.action === "gate") {
            setAutonomous((s) => ({ ...s, gate: parsed.command }));
            setStatus(`gate → ${parsed.command}`);
            return;
          }
          {
            const next = enableAutonomous(
              autonomousRef.current,
              parsed.maxTurns,
              parsed.gate,
            );
            setAutonomous(next);
            setStatus(
              `autonomous on · ${next.maxTurns} turns${
                next.gate ? ` · gate ${next.gate}` : ""
              }`,
            );
          }
          return;
        }
        case "provider":
        case "config":
          setOverlay({ screen: "configure", intent: "provider" });
          return;
        case "extensions":
          setOverlay({ screen: "extensions" });
          return;
        case "help":
          showInfo("commands & keys", formatHelpText());
          return;
        case "usage": {
          let chars = 0;
          for (const t of turns) {
            chars += t.userText.length;
            for (const item of t.responseItems) {
              if (
                item.itemType === "content_chunk" &&
                item.content.type === "text"
              ) {
                chars += item.content.text.length;
              }
            }
          }
          const auto = autonomousRef.current;
          showInfo(
            "usage",
            [
              `mode     ${MODE_LABEL[sessionModeRef.current]}`,
              `model    ${modelName || "(unset)"}`,
              `turns    ${turns.length}`,
              `tokens   ${chars ? formatTokenCount(estimateTokensFromChars(chars)) : "~0"} (estimate)`,
              `auto     ${
                auto.enabled
                  ? `on ${auto.turnsUsed}/${auto.maxTurns}`
                  : "off"
              }`,
              auto.gate ? `gate     ${auto.gate}` : "",
            ]
              .filter(Boolean)
              .join("\n"),
          );
          return;
        }
        case "copy": {
          const text = lastAssistantText(turns);
          if (!text) {
            setStatus("nothing to copy");
            return;
          }
          try {
            await copyToClipboard(text);
            setStatus("copied last reply");
          } catch (e: unknown) {
            setStatus(`copy failed: ${formatError(e)}`);
          }
          return;
        }
        case "export": {
          const dest = args
            ? resolveUserPath(args, projectCwd)
            : defaultExportPath(projectCwd);
          try {
            writeTranscript(dest, formatTranscript(turns));
            setStatus(`exported ${formatHomePath(dest)}`);
          } catch (e: unknown) {
            setStatus(`export failed: ${formatError(e)}`);
          }
          return;
        }
        case "status": {
          const client = clientRef.current;
          let provider = "";
          let model = "";
          if (client) {
            try {
              const p = await client.goose.GooseConfigRead({
                key: "GOOSE_PROVIDER",
              });
              const m = await client.goose.GooseConfigRead({
                key: "GOOSE_MODEL",
              });
              provider = typeof p.value === "string" ? p.value : "";
              model = typeof m.value === "string" ? m.value : "";
            } catch {
              // still show cwd
            }
          }
          const sid = sessionIdRef.current ?? "(none)";
          showInfo(
            "status",
            [
              `project   ${projectFolderName(projectCwd)}`,
              `path      ${formatHomePath(projectCwd)}`,
              `provider  ${provider || "(unset)"}`,
              `model     ${model || "(unset)"}`,
              `mode      ${MODE_LABEL[sessionModeRef.current]}`,
              `auto      ${
                autonomousRef.current.enabled
                  ? `on ${autonomousRef.current.turnsUsed}/${autonomousRef.current.maxTurns}`
                  : "off"
              }`,
              `session   ${sid}`,
              `status    ${status}`,
            ].join("\n"),
          );
          return;
        }
        case "clear":
          if (loadingRef.current || isProcessingRef.current) {
            await interruptTurn({ drain: true });
          }
          drainQueue();
          setAutonomous(idleAutonomous());
          setTurns([]);
          setViewTurnIdx(-1);
          setBannerVisible(true);
          setSplashGen((g) => g + 1);
          setMobiusFrame(0);
          if (clientRef.current) await createSession(clientRef.current);
          return;
        case "compact":
          if (loadingRef.current || isProcessingRef.current) {
            setStatus("stop the turn first (esc)");
            return;
          }
          setStatus("compacting…");
          await sendPrompt("/compact");
          return;
        case "diff":
          showInfo("diff", await gitDiffSummary(projectCwd));
          return;
        case "cd": {
          if (!args) {
            showInfo("cd", "Usage: /cd <path>");
            return;
          }
          const next = resolveUserPath(args, projectCwd);
          if (!existsSync(next)) {
            setStatus(`cd failed: ${next} does not exist`);
            return;
          }
          const client = clientRef.current;
          const sid = sessionIdRef.current;
          if (client && sid) {
            try {
              await client.goose.GooseWorkingDirUpdate({
                sessionId: sid,
                workingDir: next,
              });
            } catch (e: unknown) {
              setStatus(`cd failed: ${formatError(e)}`);
              return;
            }
          }
          setProjectCwd(next);
          setStatus(`cwd → ${formatHomePath(next)}`);
          return;
        }
        case "quit":
          exit();
          return;
        default:
          setStatus(`unknown: /${cmd.name}`);
      }
    },
    [
      applyModelName,
      applySessionMode,
      createSession,
      drainQueue,
      exit,
      interruptTurn,
      modelName,
      projectCwd,
      sendPrompt,
      showInfo,
      status,
      turns,
    ],
  );

  const dispatchPrompt = useCallback(
    (value: string, kind: "steer" | "followup") => {
      const trimmed = value.trim();
      if (!trimmed) return;

      if (isSlashMenuOpen(trimmed) && slashMatches.length > 0) {
        const cmd = slashMatches[slashIdx] ?? slashMatches[0]!;
        void runSlash(cmd, "");
        return;
      }

      const parsed = parseSlashInput(trimmed);
      if (parsed) {
        const cmd = resolveSlashCommand(parsed.name);
        if (cmd) {
          void runSlash(cmd, parsed.args);
          return;
        }
        setInput("");
        setStatus(`unknown command: /${parsed.name}  —  /help`);
        return;
      }

      const bang = parseBang(trimmed);
      if (bang) {
        setInput("");
        setPastedFull(null);
        void (async () => {
          setStatus("shell…");
          const result = await runShellCommand(
            bang.command,
            projectCwdRef.current,
          );
          const text = formatShellPrompt(
            bang.command,
            result.output,
            result.ok,
          );
          if (!bang.sendToModel) {
            showInfo(`$ ${bang.command}`, result.output || "(no output)");
            setStatus(result.ok ? "ready" : "shell failed");
            return;
          }
          const busyNow = loadingRef.current || isProcessingRef.current;
          if (busyNow) {
            enqueue({ text, kind });
            if (kind === "steer") void interruptTurn();
            return;
          }
          void sendPrompt(text);
        })();
        return;
      }

      setInput("");
      setPastedFull(null);
      setViewTurnIdx(-1);
      setToolCallsExpanded(false);
      setScrollOffset(0);

      const busy = loadingRef.current || isProcessingRef.current;
      if (busy) {
        enqueue({ text: trimmed, kind });
        if (kind === "steer") void interruptTurn();
        return;
      }
      void sendPrompt(trimmed);
    },
    [enqueue, interruptTurn, runSlash, sendPrompt, showInfo, slashIdx, slashMatches],
  );

  const handleSubmit = useCallback(
    (value: string) => dispatchPrompt(value, "steer"),
    [dispatchPrompt],
  );

  const handleFollowUp = useCallback(
    (value: string) => dispatchPrompt(value, "followup"),
    [dispatchPrompt],
  );

  useInput(
    (ch, key) => {
      if (key.escape) {
        if (pendingPermission) {
          resolvePermission("cancelled");
          return;
        }
        if (pastedFull !== null) return;
        if (slashOpen) {
          setInput("");
          return;
        }
        if (atOpen) {
          setInput(stripAtQuery(input));
          return;
        }
        if (
          loadingRef.current ||
          isProcessingRef.current ||
          queueRef.current.length > 0 ||
          autonomousRef.current.enabled
        ) {
          void interruptTurn({ drain: true });
        }
        return;
      }

      if ((ch === "c" || ch === "d") && key.ctrl) {
        if (pendingPermission) {
          resolvePermission("cancelled");
          return;
        }
        const running = loadingRef.current || isProcessingRef.current;
        if (running && !cancelRequestedRef.current) {
          void interruptTurn({ drain: true });
          return;
        }
        exit();
        return;
      }

      if (ch === "l" && key.ctrl) {
        setScrollOffset(0);
        setViewTurnIdx(-1);
        return;
      }

      if ((ch === "o" || ch === "O") && key.ctrl) {
        setToolCallsExpanded((prev) => !prev);
        return;
      }

      if (key.alt && key.upArrow) {
        const last = queueRef.current.pop();
        if (last) {
          setQueuedMessages([...queueRef.current]);
          if (input.trim()) {
            enqueue({ text: input, kind: "followup" });
          }
          setInput(last.text);
        }
        return;
      }

      if (slashOpen && slashMatches.length > 0) {
        if (key.upArrow && !key.shift) {
          setSlashIdx((i) => Math.max(i - 1, 0));
          return;
        }
        if (key.downArrow && !key.shift) {
          setSlashIdx((i) => Math.min(i + 1, slashMatches.length - 1));
          return;
        }
        if (key.tab && !key.shift && !key.meta && !key.ctrl) {
          const cmd = slashMatches[slashIdx] ?? slashMatches[0];
          if (cmd) setInput(`/${cmd.name} `);
          return;
        }
      }

      if (atOpen) {
        if (key.upArrow && !key.shift) {
          setAtIdx((i) => Math.max(i - 1, 0));
          return;
        }
        if (key.downArrow && !key.shift) {
          setAtIdx((i) => Math.min(i + 1, Math.max(atMatches.length - 1, 0)));
          return;
        }
        if (
          (key.tab && !key.shift && !key.meta && !key.ctrl) ||
          (key.return && !key.alt && !key.ctrl)
        ) {
          const file =
            atMatches[Math.min(atIdx, Math.max(atMatches.length - 1, 0))];
          if (file && at) setInput(applyAtMention(input, at.start, file));
          return;
        }
      }

      if (!pendingPermission && sessionIdRef.current) {
        if (key.ctrl && (ch === "p" || ch === "P")) {
          setOverlay({ screen: "configure", intent: "provider" });
          return;
        }
        if (key.ctrl && (ch === "m" || ch === "M")) {
          setOverlay({ screen: "configure", intent: "model" });
          return;
        }
        if (key.ctrl && (ch === "e" || ch === "E")) {
          setOverlay({ screen: "extensions" });
          return;
        }
        if (ch === "g" && key.ctrl) {
          setOverlay({ screen: "configure", intent: "provider" });
          return;
        }
      }

      if (pendingPermission) {
        const opts = pendingPermission.options;
        if (key.upArrow) {
          setPermissionIdx((i) => (i - 1 + opts.length) % opts.length);
          return;
        }
        if (key.downArrow) {
          setPermissionIdx((i) => (i + 1) % opts.length);
          return;
        }
        if (key.return) {
          const sel = opts[permissionIdx];
          if (sel) resolvePermission({ optionId: sel.optionId });
          return;
        }
        const keyMap: Record<string, string> = {
          y: "allow_once",
          a: "allow_always",
          n: "reject_once",
          N: "reject_always",
        };
        const kind = keyMap[ch];
        if (kind) {
          const m = opts.find((o) => o.kind === kind);
          if (m) resolvePermission({ optionId: m.optionId });
        }
        return;
      }

      const viewingHistory =
        viewTurnIdx !== -1 && viewTurnIdx < turns.length - 1;
      const multilineOwnsArrows =
        !pendingPermission &&
        !initialPrompt &&
        !viewingHistory &&
        pastedFull === null;

      if (key.tab && (key.shift || key.meta || key.ctrl)) {
        if (!pendingPermission && !slashOpen && !atOpen) cycleSessionMode();
        return;
      }

      if (key.tab) {
        const idx = viewTurnIdx === -1 ? turns.length - 1 : viewTurnIdx;
        const t = turns[idx];
        if (t && t.responseItems.some((it) => it.itemType === "tool_call")) {
          setToolCallsExpanded((prev) => !prev);
        }
        return;
      }

      if (key.upArrow && !key.shift) {
        if (!multilineOwnsArrows) setScrollOffset((prev) => prev + 3);
        return;
      }
      if (key.downArrow && !key.shift) {
        if (!multilineOwnsArrows)
          setScrollOffset((prev) => Math.max(prev - 3, 0));
        return;
      }

      if (key.upArrow && key.shift) {
        setTurns((cur) => {
          if (cur.length <= 1) return cur;
          setViewTurnIdx((prev) => {
            const eff = prev === -1 ? cur.length - 1 : prev;
            return Math.max(eff - 1, 0);
          });
          return cur;
        });
        return;
      }
      if (key.downArrow && key.shift) {
        setTurns((cur) => {
          if (cur.length <= 1) return cur;
          setViewTurnIdx((prev) => {
            if (prev === -1) return -1;
            const next = prev + 1;
            return next >= cur.length ? -1 : next;
          });
          return cur;
        });
        return;
      }
    },
    { isActive: !needsOnboarding && !overlay },
  );

  const PAD_X = 2;
  const PAD_Y = 1;
  const safeTermWidth = Math.max(termWidth, 40);
  const safeTermHeight = Math.max(termHeight, 10);
  const contentWidth = Math.max(safeTermWidth - PAD_X * 2, 20);

  const effectiveTurnIdx = viewTurnIdx === -1 ? turns.length - 1 : viewTurnIdx;
  const currentTurn = turns[effectiveTurnIdx];
  const isViewingHistory = viewTurnIdx !== -1 && viewTurnIdx < turns.length - 1;
  const isLatest = !isViewingHistory;
  const showInputBar =
    !pendingPermission && !initialPrompt && !isViewingHistory;

  const headerH = 3;
  const isPasteMode = pastedFull !== null;
  const inputContentRows = showInputBar
    ? isPasteMode
      ? 1
      : Math.min(Math.max(input.split("\n").length, 1), INPUT_MAX_ROWS)
    : 0;
  const inputExtraLines = showInputBar ? 1 : 0;
  const slashH =
    showInputBar && slashOpen ? slashMenuHeight(slashMatches.length) : 0;
  const atH =
    showInputBar && atOpen ? pickListHeight(atMatches.length) : 0;
  const inputBarH = showInputBar
    ? 2 + inputContentRows + inputExtraLines + slashH + atH
    : 0;
  const historyBarH = isViewingHistory ? 2 : 0;
  const footerH = showInputBar ? 1 : 0;
  const viewportHeight = Math.max(
    safeTermHeight - PAD_Y * 2 - headerH - inputBarH - historyBarH - footerH,
    3,
  );

  const tokensLabel = useMemo(() => {
    let chars = 0;
    for (const t of turns) {
      chars += t.userText.length;
      for (const item of t.responseItems) {
        if (
          item.itemType === "content_chunk" &&
          item.content.type === "text"
        ) {
          chars += item.content.text.length;
        }
      }
    }
    return chars > 0 ? formatTokenCount(estimateTokensFromChars(chars)) : "";
  }, [turns]);

  const contentLines = useMemo(
    () =>
      buildContentLines({
        turn: currentTurn,
        turnIndex: effectiveTurnIdx,
        width: contentWidth,
        loading: isLatest && loading,
        status,
        spinIdx,
        pendingPermission: isLatest ? pendingPermission : null,
        permissionIdx,
        toolCallsExpanded,
        queuedMessages: isLatest ? queuedMessages : [],
      }),
    [
      currentTurn,
      effectiveTurnIdx,
      contentWidth,
      isLatest,
      loading,
      status,
      spinIdx,
      pendingPermission,
      permissionIdx,
      toolCallsExpanded,
      queuedMessages,
    ],
  );

  if (needsOnboarding && clientRef.current) {
    return (
      <Box flexDirection="column" width={safeTermWidth} height={safeTermHeight}>
        <Onboarding
          client={clientRef.current}
          width={safeTermWidth}
          height={safeTermHeight}
          onComplete={handleOnboardingComplete}
        />
      </Box>
    );
  }

  if (overlay) {
    if (overlay.screen === "info") {
      return (
        <Box
          flexDirection="column"
          width={safeTermWidth}
          height={safeTermHeight}
        >
          <InfoOverlay
            title={overlay.title}
            body={overlay.body}
            width={safeTermWidth}
            height={safeTermHeight}
            onClose={() => setOverlay(null)}
          />
        </Box>
      );
    }
    if (clientRef.current && sessionIdRef.current) {
    if (overlay.screen === "configure") {
      const intent = overlay.intent;
      return (
        <Box
          flexDirection="column"
          width={safeTermWidth}
          height={safeTermHeight}
        >
          <ConfigureScreen
            client={clientRef.current}
            sessionId={sessionIdRef.current}
            width={safeTermWidth}
            height={safeTermHeight}
            onComplete={() => {
              setOverlay(null);
              setStatus("ready");
            }}
            onCancel={() => setOverlay(null)}
            initialIntent={intent}
          />
        </Box>
      );
    } else if (overlay.screen === "extensions") {
      return (
        <Box
          flexDirection="column"
          width={safeTermWidth}
          height={safeTermHeight}
        >
          <ExtensionsManager
            client={clientRef.current}
            sessionId={sessionIdRef.current}
            height={safeTermHeight}
            onClose={() => setOverlay(null)}
          />
        </Box>
      );
    }
    }
  }

  return (
    <Box
      flexDirection="column"
      width={safeTermWidth}
      height={safeTermHeight}
      paddingX={PAD_X}
      paddingY={PAD_Y}
    >
      {bannerVisible ? (
        <SplashScreen
          width={contentWidth}
          height={Math.max(safeTermHeight - PAD_Y * 2 - inputBarH - footerH, 0)}
          status={status}
          loading={loading}
          spinIdx={spinIdx}
          projectName={projectName}
          projectPath={projectPath}
          contextLine={contextLine}
          mobiusFrame={mobiusFrame}
        />
      ) : (
        <>
          <Header
            width={contentWidth}
            status={status}
            loading={loading}
            spinIdx={spinIdx}
            hasPendingPermission={!!pendingPermission}
            projectName={projectName}
            projectPath={projectPath}
            turnInfo={
              turns.length > 1
                ? { current: effectiveTurnIdx + 1, total: turns.length }
                : undefined
            }
          />

          <Viewport
            lines={contentLines}
            height={viewportHeight}
            width={contentWidth}
            scrollOffset={scrollOffset}
          />

          {isViewingHistory && (
            <Box flexDirection="column" width={contentWidth} flexShrink={0}>
              <Rule width={contentWidth} />
              <Box justifyContent="center" width={contentWidth}>
                <Text color={GOLD}>
                  turn {effectiveTurnIdx + 1}/{turns.length}
                </Text>
                <Text color={TEXT_DIM}> — shift+↓ to return</Text>
              </Box>
            </Box>
          )}
        </>
      )}
      {showInputBar && (
        <>
          {slashOpen && (
            <SlashMenu
              width={contentWidth}
              matches={slashMatches}
              selectedIndex={slashIdx}
            />
          )}
          {atOpen && (
            <FileMenu
              width={contentWidth}
              files={atMatches}
              selectedIndex={atIdx}
            />
          )}
          <InputBar
          width={contentWidth}
          input={input}
          onChange={setInput}
          onSubmit={handleSubmit}
          onFollowUp={handleFollowUp}
          queued={queuedMessages.length > 0}
          queuedKind={queuedMessages[0]?.kind ?? null}
          busy={loading && turns.length > 0}
          autonomous={autonomous.enabled}
          scrollHint={!bannerVisible && turns.length > 1}
          placeholder={bannerVisible ? INITIAL_GREETING : undefined}
          focused={showInputBar}
          pastedFull={pastedFull}
          onPastedFullChange={setPastedFull}
          slashOpen={slashOpen}
          atOpen={atOpen}
        />
          <Footer
            width={contentWidth}
            mode={sessionMode}
            model={modelName}
            tokensLabel={tokensLabel}
            autonomousLabel={
              autonomous.enabled
                ? `${autonomous.turnsUsed}/${autonomous.maxTurns}`
                : null
            }
          />
        </>
      )}
    </Box>
  );
}

const cli = meow(
  `
  Usage
    $ permagent

  Options
    --server, -s  Server URL (default: auto-launch bundled server)
    --text, -t    Send a single prompt and exit
`,
  {
    importMeta: import.meta,
    flags: {
      server: { type: "string", shortFlag: "s" },
      text: { type: "string", shortFlag: "t" },
    },
  },
);

let serverProcess: ReturnType<typeof spawn> | null = null;

async function runTextMode(serverConnection: Stream | string, prompt: string) {
  try {
    const client = new GooseClient(
      () => ({
        sessionUpdate: async (params: SessionNotification) => {
          const update = params.update;
          if (update.sessionUpdate === "agent_message_chunk") {
            if (update.content.type === "text") {
              process.stdout.write(update.content.text);
            }
          }
        },
        requestPermission: async (
          params: RequestPermissionRequest,
        ): Promise<RequestPermissionResponse> => {
          // Auto-reject in text mode
          const rejectOption = params.options.find(
            (o) => o.kind === "reject_once",
          );
          if (rejectOption) {
            return {
              outcome: { outcome: "selected", optionId: rejectOption.optionId },
            };
          }
          return { outcome: { outcome: "cancelled" } };
        },
      }),
      serverConnection,
    );

    await client.initialize({
      protocolVersion: 0,
      clientInfo: { name: "permagent-tui", version: "0.1.0" },
      clientCapabilities: {},
    });

    const session = await client.newSession({
      cwd: process.cwd(),
      mcpServers: [],
    });

    await client.prompt({
      sessionId: session.sessionId,
      prompt: [{ type: "text", text: prompt }],
    });

    process.stdout.write("\n");
  } catch (e: unknown) {
    const errMsg = e instanceof Error ? e.message : String(e);
    console.error(`Error: ${errMsg}`);
    process.exit(1);
  }
}

async function main() {
  let serverConnection: Stream | string;

  if (cli.flags.server) {
    serverConnection = cli.flags.server;
  } else {
    const binary = resolveGooseBinary();
    serverProcess = spawn(binary, ["acp"], {
      stdio: ["pipe", "pipe", "ignore"],
      detached: false,
    });

    serverProcess.on("error", (err) => {
      console.error(`Failed to start permagent ACP: ${err.message}`);
      process.exit(1);
    });

    const output = Writable.toWeb(
      serverProcess.stdin!,
    ) as WritableStream<Uint8Array>;
    const input = Readable.toWeb(
      serverProcess.stdout!,
    ) as ReadableStream<Uint8Array>;
    serverConnection = ndJsonStream(output, input);
  }

  // Text mode: bypass TUI and stream directly to stdout
  if (cli.flags.text) {
    await runTextMode(serverConnection, cli.flags.text);
    cleanup();
    return;
  }

  // Interactive TUI mode
  const { waitUntilExit } = render(
    <App serverConnection={serverConnection} initialPrompt={cli.flags.text} />,
  );

  await waitUntilExit();
  cleanup();
}

function cleanup() {
  if (serverProcess && !serverProcess.killed) {
    serverProcess.kill();
  }
}

process.on("exit", cleanup);
process.on("SIGINT", () => {
  cleanup();
  process.exit(0);
});
process.on("SIGTERM", () => {
  cleanup();
  process.exit(0);
});

main().catch((err) => {
  console.error(err);
  cleanup();
  process.exit(1);
});
