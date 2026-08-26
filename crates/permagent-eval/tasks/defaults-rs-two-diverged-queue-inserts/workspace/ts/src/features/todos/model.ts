import type { Todo } from "../../core/types.ts";

export function createTodo(id: string, title: string, createdAt: number): Todo {
  return { id, title, done: false, createdAt };
}

export function completeTodo(todo: Todo): Todo {
  return { ...todo, done: true };
}
