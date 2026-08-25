import { h } from "../lib/h.ts";
import type { VNode } from "../lib/h.ts";

export function Header(title: string): VNode {
  return h("header", {}, h("h1", {}, title));
}
