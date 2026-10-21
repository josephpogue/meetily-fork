import type { AssistantTodo, AssistantTodoOwner } from '@/types';

/** One owner's to-dos, split into still open and done; order is the input's (newest first). */
export interface OwnerTodos {
  open: AssistantTodo[];
  done: AssistantTodo[];
}

export function splitByOwner(todos: AssistantTodo[]): Record<AssistantTodoOwner, OwnerTodos> {
  const out: Record<AssistantTodoOwner, OwnerTodos> = {
    you: { open: [], done: [] },
    them: { open: [], done: [] },
  };
  for (const t of todos) out[t.owner][t.done ? 'done' : 'open'].push(t);
  return out;
}

/** The newest open to-do, or null. The backend delivers newest first. */
export function newestOpen(todos: AssistantTodo[]): AssistantTodo | null {
  return todos.find((t) => !t.done) ?? null;
}

/** New = arrived after the last time Joseph closed the drawer (0 until he first does). */
export function isNewTodo(todo: AssistantTodo, lastClosedAt: number): boolean {
  return todo.at > lastClosedAt;
}

/**
 * "40:05" for a call time in seconds: the transcript's own stamp (see
 * formatRecordingTime in VirtualizedTranscriptView) without its brackets. Mirrored
 * here, keep in lockstep: importing that component pulls the transcript hooks and
 * contexts into every test that loads this file.
 */
export function callStamp(seconds: number): string {
  const total = Math.floor(seconds);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
}

/** "By Friday" stays as said; "monday" gets its first letter capitalised. */
export function capitalise(text: string): string {
  return text ? text[0].toUpperCase() + text.slice(1) : text;
}

// A leading "todo:" (any case, leading spaces allowed) turns the Ask box into a to-do entry.
const TODO_PREFIX = /^\s*todo:\s?/i;

export function isTodoEntry(value: string): boolean {
  return TODO_PREFIX.test(value);
}

/** The text after the todo: prefix, or the value unchanged when there is none. */
export function stripTodoPrefix(value: string): string {
  return value.replace(TODO_PREFIX, '');
}
