import { h } from "../lib/h.ts";
import type { VNode } from "../lib/h.ts";
import type { Todo } from "../core/types.ts";

export function TodoItem(todo: Todo): VNode {
  return h("li", { className: todo.done ? "done" : "" }, todo.title);
}
