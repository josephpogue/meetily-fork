'use client';

import { useEffect, useRef } from 'react';
import { X } from 'lucide-react';
import { cn } from '@/lib/utils';
import type { AssistantTodo, AssistantTodoOwner } from '@/types';
import { callStamp, capitalise, isNewTodo, splitByOwner } from '@/lib/todos';
import { NewTag, OwnerPill } from './TodoStrip';
import { ItemCheckbox, useRowSelection } from './selection';

interface TodoDrawerColumnsProps {
  todos: AssistantTodo[];
  lastClosedAt: number;
  onDismiss: (id: string) => void;
  /** Where focus goes when a dismiss empties the column it was in. */
  focusClose: () => void;
}

/**
 * The drawer's two columns. They use the lanes grid split, so "You promised"
 * sits under the Say lane and "They owe you" under the Ask lane. Each column
 * scrolls on its own.
 */
export function TodoDrawerColumns({ todos, lastClosedAt, onDismiss, focusClose }: TodoDrawerColumnsProps) {
  const split = splitByOwner(todos);
  return (
    <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1.08fr)_minmax(0,1fr)]">
      <Column owner="you" title="You promised" group={split.you} lastClosedAt={lastClosedAt} onDismiss={onDismiss} focusClose={focusClose} list={todos} />
      <Column
        owner="them"
        title="They owe you"
        group={split.them}
        lastClosedAt={lastClosedAt}
        onDismiss={onDismiss}
        focusClose={focusClose}
        list={todos}
        className="border-l border-gray-200"
      />
    </div>
  );
}

interface ColumnProps {
  owner: AssistantTodoOwner;
  title: string;
  group: { open: AssistantTodo[]; done: AssistantTodo[] };
  lastClosedAt: number;
  onDismiss: (id: string) => void;
  focusClose: () => void;
  /** The whole to-do list as last delivered: a new array means a new update arrived. */
  list: AssistantTodo[];
  className?: string;
}

function Column({ owner, title, group, lastClosedAt, onDismiss, focusClose, list, className }: ColumnProps) {
  // After a dismiss the row is gone and focus would fall to the page. Once the
  // item has left the list, focus goes to the next row's dismiss button, else
  // the previous one, else the drawer's Close button.
  const buttons = useRef(new Map<string, HTMLButtonElement | null>());
  // `after` is the list as it must look once the dismiss has landed; any other
  // update (the item stayed, or the list was reset for a new meeting) drops it.
  const pending = useRef<{ id: string; target: string | null; after: string } | null>(null);
  const order = [...group.open, ...group.done].map((t) => t.id);
  const dismiss = (id: string) => {
    const at = order.indexOf(id);
    pending.current = {
      id,
      target: order[at + 1] ?? order[at - 1] ?? null,
      after: order.filter((x) => x !== id).join('|'),
    };
    onDismiss(id);
  };
  const orderKey = order.join('|');
  useEffect(() => {
    const p = pending.current;
    if (!p) return;
    pending.current = null;
    if (orderKey !== p.after) return;
    if (p.target === null) focusClose();
    else buttons.current.get(p.target)?.focus?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list]);
  const setButton = (id: string) => (el: HTMLButtonElement | null) => {
    if (el) buttons.current.set(id, el);
    else buttons.current.delete(id);
  };
  const empty = owner === 'you' ? 'Nothing promised yet' : 'Nothing owed yet';
  return (
    <section aria-label={title} className={cn('min-h-0 min-w-0 overflow-y-auto px-3 pb-3', className)}>
      <div className="flex items-center gap-2 pb-1 pt-2.5">
        <OwnerPill owner={owner} label={owner === 'you' ? 'You' : 'Them'} />
        <h3 className="text-xs font-semibold text-gray-700">{title}</h3>
        <span className="text-xs tabular-nums text-gray-500">{group.open.length}</span>
      </div>
      {group.open.length === 0 ? (
        <div className="py-2">
          <p className="text-xs text-gray-500">{empty}</p>
          {owner === 'you' && <p className="mt-0.5 text-xs text-gray-400">Type todo: in the Ask box to add one.</p>}
        </div>
      ) : (
        <ul className="flex flex-col">
          {group.open.map((t) => (
            <TodoRow key={t.id} todo={t} fresh={isNewTodo(t, lastClosedAt)} onDismiss={dismiss} buttonRef={setButton(t.id)} />
          ))}
        </ul>
      )}
      {group.done.length > 0 && (
        <>
          <div className="flex items-center gap-2 pb-1 pt-4">
            <h4 className="text-xs font-semibold text-gray-500">Done</h4>
            <span className="text-xs tabular-nums text-gray-500">{group.done.length}</span>
          </div>
          <ul className="flex flex-col">
            {group.done.map((t) => (
              <TodoRow key={t.id} todo={t} fresh={false} onDismiss={dismiss} buttonRef={setButton(t.id)} />
            ))}
          </ul>
        </>
      )}
    </section>
  );
}

function TodoRow({
  todo,
  fresh,
  onDismiss,
  buttonRef,
}: {
  todo: AssistantTodo;
  fresh: boolean;
  onDismiss: (id: string) => void;
  buttonRef: (el: HTMLButtonElement | null) => void;
}) {
  const row = useRowSelection(todo.id);
  const done = todo.done;
  const origin = `${todo.origin === 'typed' ? 'You added at' : 'Said at'} ${callStamp(todo.callTime)}`;
  return (
    <li
      {...row.rowProps}
      className={cn('group flex items-start gap-2 border-t border-gray-100 py-2 first:border-t-0', row.rowClass)}
    >
      {row.selecting && (
        <span className="mt-0.5 grid h-5 w-4 shrink-0 place-items-center">
          <ItemCheckbox id={todo.id} label={`Select to-do: ${todo.text}`} />
        </span>
      )}
      <div className="min-w-0 flex-1">
        <p className={cn('break-words text-sm leading-snug', done ? 'text-gray-500' : 'font-medium text-gray-900')}>
          {todo.text}
          {fresh && <NewTag />}
        </p>
        <p className="mt-0.5 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-snug text-gray-500">
          {done ? (
            <span className="tabular-nums">{`Done at ${callStamp(done.callTime)} · ${done.how}`}</span>
          ) : (
            <>
              {todo.owner === 'them' && <OwnerPill owner="them" label={todo.who || 'Them'} className="mr-0.5" />}
              {todo.due && (
                <>
                  <span className="font-semibold text-gray-700">{capitalise(todo.due)}</span>
                  <span aria-hidden="true" className="text-gray-300">&middot;</span>
                </>
              )}
              <span className="tabular-nums">{origin}</span>
            </>
          )}
        </p>
      </div>
      {!row.selecting && (
        <button
          ref={buttonRef}
          type="button"
          onClick={() => onDismiss(todo.id)}
          title="Dismiss"
          aria-label={`Dismiss to-do: ${todo.text}`}
          className="mt-0.5 shrink-0 rounded text-gray-300 opacity-0 hover:text-gray-500 focus-visible:opacity-100 focus-visible:text-gray-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rose-400 group-hover:opacity-100"
        >
          <X className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      )}
    </li>
  );
}
