import React from "react";
import { Box, Text } from "ink";
import { Spinner } from "./Spinner.js";
import { Rule } from "./Rule.js";
import { TEAL, CRANBERRY, TEXT_PRIMARY, TEXT_DIM, RULE_COLOR } from "../colors.js";
import { isErrorStatus } from "../utils.js";

interface HeaderProps {
  width: number;
  status: string;
  loading: boolean;
  spinIdx: number;
  hasPendingPermission: boolean;
  projectName: string;
  projectPath: string;
  turnInfo?: { current: number; total: number };
}

export const Header = React.memo(function Header({
  width,
  status,
  loading,
  spinIdx,
  hasPendingPermission,
  projectName,
  projectPath,
  turnInfo,
}: HeaderProps) {
  const statusColor =
    status === "ready" ? TEAL : isErrorStatus(status) ? CRANBERRY : TEXT_DIM;

  const constrainedWidth = Math.max(width, 20);
  const rightReserve = 22;
  const leftWidth = Math.max(constrainedWidth - rightReserve, 10);

  return (
    <Box flexDirection="column" width={constrainedWidth} flexShrink={0}>
      <Box justifyContent="space-between" width={constrainedWidth}>
        <Box width={leftWidth}>
          <Text color={TEXT_PRIMARY} bold>
            permagent
          </Text>
          <Text color={RULE_COLOR}> · </Text>
          <Box flexShrink={1}>
            <Text color={TEXT_PRIMARY} wrap="truncate-end">
              {projectName}
            </Text>
          </Box>
        </Box>
        <Box width={Math.min(rightReserve, constrainedWidth - leftWidth)} justifyContent="flex-end">
          {turnInfo && turnInfo.total > 1 && (
            <Text color={TEXT_DIM}>
              {turnInfo.current}/{turnInfo.total}{"  "}
            </Text>
          )}
          <Text color={statusColor}>{status}</Text>
          {loading && !hasPendingPermission && (
            <Text>
              {" "}
              <Spinner idx={spinIdx} />
            </Text>
          )}
        </Box>
      </Box>
      <Box width={constrainedWidth}>
        <Text color={TEXT_DIM} wrap="truncate-start">
          {projectPath}
        </Text>
      </Box>
      <Rule width={constrainedWidth} />
    </Box>
  );
});
