import { h } from "../lib/h.ts";
import type { VNode } from "../lib/h.ts";

export function Button(label: string, onClick: () => void): VNode {
  return h("button", { onClick, type: "button" }, label);
}
