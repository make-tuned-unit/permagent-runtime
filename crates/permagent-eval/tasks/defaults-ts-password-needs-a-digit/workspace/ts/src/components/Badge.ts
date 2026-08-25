import { h } from "../lib/h.ts";
import type { VNode } from "../lib/h.ts";

export function Badge(text: string, tone: "neutral" | "success" | "danger" = "neutral"): VNode {
  return h("span", { className: `badge badge-${tone}` }, text);
}
