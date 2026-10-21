'use client';

import type { Ref } from 'react';
import { ChevronDown, ChevronUp, ClipboardList } from 'lucide-react';
import { cn } from '@/lib/utils';
import type { AssistantTodo, AssistantTodoOwner } from '@/types';
import { capitalise, isNewTodo, newestOpen, splitByOwner } from '@/lib/todos';
import { TriCheckbox, useGroupState, useSelection } from './selection';

/**
 * You is Joseph's own promise (solid rose); Them is what the other side owes
 * him (white with a grey ring). Both the fill and the colour change, so the
 * two read apart without relying on hue.
 */
export function OwnerPill({ owner, label, className }: { owner: AssistantTodoOwner; label?: string; className?: string }) {
  return (
    <span
      className={cn(
        'inline-flex h-[18px] shrink-0 items-center justify-center whitespace-nowrap rounded-full px-2 text-[11px] font-bold leading-none',
        owner === 'you' ? 'bg-rose-600 text-white' : 'bg-white text-gray-600 ring-1 ring-inset ring-gray-300',
        className
      )}
    >
      {label ?? (owner === 'you' ? 'You' : 'Them')}
    </span>
  );
}

export function NewTag() {
  return <span className="ml-1 text-[11px] font-bold text-rose-700">New</span>;
}

interface TodoStripProps {
  todos: AssistantTodo[];
  lastClosedAt: number;
  open: boolean;
  onToggle: () => void;
  buttonRef?: Ref<HTMLButtonElement>;
  /** The strip under an open drawer is kept for its height only. */
  covered?: boolean;
}

/**
 * The one-line To do strip: counts per owner and the newest open to-do in
 * full. Closed, it sits above Explain and Catch up; open, the same bar is the
 * drawer's header. The whole bar toggles; the chevron is the keyboard target.
 */
export function TodoStrip({ todos, lastClosedAt, open, onToggle, buttonRef, covered = false }: TodoStripProps) {
  const split = splitByOwner(todos);
  const newest = newestOpen(todos);
  const label = open ? 'Close to-do list' : 'Open to-do list';
  const Chevron = open ? ChevronDown : ChevronUp;
  const { selecting, setMany } = useSelection();
  const ids = todos.map((t) => t.id);
  const group = useGroupState(ids);

  let summary: React.ReactNode;
  if (selecting) {
    // Selections hidden in a closed drawer stay visible here.
    summary = (
      <span className="text-[12.5px] font-semibold text-slate-700">{`${group.count} of ${ids.length} selected`}</span>
    );
  } else if (open) {
    summary = 'Newest first';
  } else if (!newest) {
    summary = todos.length === 0 ? 'Nothing yet' : 'Nothing open';
  } else {
    summary = (
      <>
        {isNewTodo(newest, lastClosedAt) && <b className="mr-1 font-bold text-rose-700">New</b>}
        {newest.text}
        {newest.due ? `, ${capitalise(newest.due)}` : ''}
      </>
    );
  }

  return (
    <div
      onClick={onToggle}
      aria-hidden={covered || undefined}
      className={cn(
        'flex w-full shrink-0 cursor-pointer items-center gap-2 px-3.5 py-2 text-left',
        open ? 'border-b border-rose-200 bg-rose-50 hover:bg-rose-100/70' : 'border-t border-gray-200 bg-white hover:bg-gray-50',
        covered && 'invisible'
      )}
    >
      <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-rose-600 text-white">
        <ClipboardList className="h-3.5 w-3.5" aria-hidden="true" />
      </span>
      <span className="shrink-0 text-sm font-semibold text-rose-700">To do</span>
      <span className="flex shrink-0 items-center gap-1">
        <OwnerPill owner="you" />
        <span className="text-xs font-semibold tabular-nums text-gray-700">{split.you.open.length}</span>
      </span>
      <span className="flex shrink-0 items-center gap-1">
        <OwnerPill owner="them" />
        <span className="text-xs font-semibold tabular-nums text-gray-700">{split.them.open.length}</span>
      </span>
      {selecting && open && (
        <TriCheckbox
          checked={group.all}
          indeterminate={group.some}
          disabled={ids.length === 0}
          onChange={() => setMany(ids, !group.all)}
          label="Select all to-dos"
        />
      )}
      <span className="min-w-0 flex-1 break-words pl-1 text-xs leading-snug text-gray-600">{summary}</span>
      <button
        type="button"
        ref={buttonRef}
        aria-label={label}
        aria-expanded={open}
        tabIndex={covered ? -1 : undefined}
        onClick={(e) => {
          e.stopPropagation();
          onToggle();
        }}
        className="shrink-0 rounded p-0.5 text-gray-500 hover:text-gray-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rose-400"
      >
        <Chevron className="h-4 w-4" aria-hidden="true" />
      </button>
    </div>
  );
}
