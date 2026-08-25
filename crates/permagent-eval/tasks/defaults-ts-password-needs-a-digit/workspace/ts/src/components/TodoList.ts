import { h } from "../lib/h.ts";
import type { VNode } from "../lib/h.ts";
import type { Todo } from "../core/types.ts";
import { TodoItem } from "./TodoItem.ts";

export function TodoList(todos: Todo[]): VNode {
  return h("ul", {}, ...todos.map(TodoItem));
}
