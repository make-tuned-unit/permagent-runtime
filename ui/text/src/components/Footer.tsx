import React from "react";
import { Box, Text } from "ink";
import { TEXT_DIM, GOLD, TEAL } from "../colors.js";
import { MODE_LABEL, type SessionMode } from "../sessionMode.js";

export function Footer({
  width,
  mode,
  model,
  tokensLabel,
  autonomousLabel,
}: {
  width: number;
  mode: SessionMode;
  model: string;
  tokensLabel: string;
  autonomousLabel: string | null;
}) {
  const constrained = Math.max(width, 20);
  const left = [
    MODE_LABEL[mode],
    autonomousLabel,
    model || null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <Box width={constrained} justifyContent="space-between" flexShrink={0}>
      <Box flexShrink={1}>
        <Text color={mode === "auto" ? TEAL : mode === "chat" ? GOLD : TEXT_DIM} wrap="truncate-end">
          {left}
        </Text>
      </Box>
      {tokensLabel ? (
        <Text color={TEXT_DIM}>{tokensLabel}</Text>
      ) : null}
    </Box>
  );
}
