import type { Todo } from "../../core/types.ts";

export function selectActive(todos: Todo[]): Todo[] {
  return todos.filter((t) => !t.done);
}

export function selectDone(todos: Todo[]): Todo[] {
  return todos.filter((t) => t.done);
}
