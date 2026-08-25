import type { Todo } from "../../core/types.ts";

export type TodoAction =
  | { type: "add"; todo: Todo }
  | { type: "toggle"; id: string }
  | { type: "remove"; id: string };

export function todosReducer(state: Todo[], action: TodoAction): Todo[] {
  switch (action.type) {
    case "add":
      return [...state, action.todo];
    case "toggle":
      return state.map((t) => (t.id === action.id ? { ...t, done: !t.done } : t));
    case "remove":
      return state.filter((t) => t.id !== action.id);
    default:
      return state;
  }
}
