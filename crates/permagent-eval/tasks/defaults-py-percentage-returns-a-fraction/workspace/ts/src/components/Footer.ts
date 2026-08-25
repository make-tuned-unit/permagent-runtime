import { h } from "../lib/h.ts";
import type { VNode } from "../lib/h.ts";

export function Footer(year: number): VNode {
  return h("footer", {}, `© ${year}`);
}
