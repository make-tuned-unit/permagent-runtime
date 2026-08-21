import React, { useState } from "react";
import { Box, Text, useInput } from "ink";
import { GOLD, TEXT_DIM, TEXT_PRIMARY, RULE_COLOR } from "../colors.js";
import { brandCopy } from "../brand.js";
import { Rule } from "./Rule.js";

export const InfoOverlay = React.memo(function InfoOverlay({
  title,
  body,
  width,
  height,
  onClose,
}: {
  title: string;
  body: string;
  width: number;
  height: number;
  onClose: () => void;
}) {
  const lines = brandCopy(body).split("\n");
  const innerHeight = Math.max(height - 6, 3);
  const maxOffset = Math.max(0, lines.length - innerHeight);
  const [offset, setOffset] = useState(0);

  useInput((ch, key) => {
    if (key.escape || key.return || ch === "q") {
      onClose();
      return;
    }
    if (key.upArrow) {
      setOffset((o) => Math.max(0, o - 1));
      return;
    }
    if (key.downArrow) {
      setOffset((o) => Math.min(maxOffset, o + 1));
    }
  });

  const visible = lines.slice(offset, offset + innerHeight);
  const constrainedWidth = Math.max(width, 20);

  return (
    <Box
      flexDirection="column"
      width={constrainedWidth}
      height={Math.max(height, 8)}
      paddingX={2}
      paddingY={1}
    >
      <Text color={TEXT_PRIMARY} bold>
        {title}
      </Text>
      <Rule width={Math.max(constrainedWidth - 4, 10)} />
      <Box flexDirection="column" height={innerHeight} flexGrow={1}>
        {visible.map((line, i) => (
          <Text key={`${offset}-${i}`} color={TEXT_DIM} wrap="truncate-end">
            {line.length === 0 ? " " : line}
          </Text>
        ))}
      </Box>
      <Box>
        <Text color={GOLD}>esc</Text>
        <Text color={RULE_COLOR}> close</Text>
        {maxOffset > 0 && <Text color={TEXT_DIM}> · ↑↓ scroll</Text>}
      </Box>
    </Box>
  );
});
