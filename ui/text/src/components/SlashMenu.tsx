import React from "react";
import { Box, Text } from "ink";
import { GOLD, TEXT_DIM, TEXT_PRIMARY, RULE_COLOR } from "../colors.js";
import type { SlashCommandDef } from "../slashCommands.js";

const MAX_VISIBLE = 8;

export function slashMenuHeight(matchCount: number): number {
  const rows = Math.min(Math.max(matchCount, 1), MAX_VISIBLE);
  return rows + 2;
}

export const SlashMenu = React.memo(function SlashMenu({
  width,
  matches,
  selectedIndex,
}: {
  width: number;
  matches: SlashCommandDef[];
  selectedIndex: number;
}) {
  const constrainedWidth = Math.max(width, 20);
  const start = Math.max(
    0,
    Math.min(selectedIndex - MAX_VISIBLE + 1, Math.max(matches.length - MAX_VISIBLE, 0)),
  );
  const visible = matches.slice(start, start + MAX_VISIBLE);
  const inner = Math.max(constrainedWidth - 4, 12);

  return (
    <Box
      flexDirection="column"
      borderStyle="round"
      borderColor={GOLD}
      width={constrainedWidth}
      flexShrink={0}
      paddingX={1}
    >
      {matches.length === 0 ? (
        <Text color={TEXT_DIM}>no matching commands</Text>
      ) : (
        visible.map((cmd, vi) => {
          const idx = start + vi;
          const active = idx === selectedIndex;
          const name = `/${cmd.name}`;
          const keys = cmd.keys ? ` ${cmd.keys}` : "";
          return (
            <Box key={cmd.name} width={inner}>
              <Text color={active ? GOLD : TEXT_PRIMARY} bold={active} wrap="truncate-end">
                {active ? "› " : "  "}
                {name.padEnd(14)}
                {cmd.summary}
                {keys}
              </Text>
            </Box>
          );
        })
      )}
      {matches.length > MAX_VISIBLE && (
        <Text color={RULE_COLOR}>tab complete · ↑↓ select</Text>
      )}
    </Box>
  );
});
