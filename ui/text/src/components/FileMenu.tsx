import React from "react";
import { Box, Text } from "ink";
import { GOLD, TEXT_DIM, TEXT_PRIMARY, RULE_COLOR } from "../colors.js";

const MAX_VISIBLE = 8;

export function pickListHeight(matchCount: number): number {
  const rows = Math.min(Math.max(matchCount, 1), MAX_VISIBLE);
  return rows + 2;
}

export const FileMenu = React.memo(function FileMenu({
  width,
  files,
  selectedIndex,
}: {
  width: number;
  files: string[];
  selectedIndex: number;
}) {
  const constrainedWidth = Math.max(width, 20);
  const start = Math.max(
    0,
    Math.min(selectedIndex - MAX_VISIBLE + 1, Math.max(files.length - MAX_VISIBLE, 0)),
  );
  const visible = files.slice(start, start + MAX_VISIBLE);
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
      {files.length === 0 ? (
        <Text color={TEXT_DIM}>no matching files</Text>
      ) : (
        visible.map((file, vi) => {
          const idx = start + vi;
          const active = idx === selectedIndex;
          return (
            <Box key={file} width={inner}>
              <Text color={active ? GOLD : TEXT_PRIMARY} bold={active} wrap="truncate-start">
                {active ? "› " : "  "}
                {file}
              </Text>
            </Box>
          );
        })
      )}
      {files.length > MAX_VISIBLE && (
        <Text color={RULE_COLOR}>tab complete · ↑↓ select</Text>
      )}
    </Box>
  );
});
