'use client';

import { useEffect, useRef } from 'react';
import { Trash2 } from 'lucide-react';
import { TriCheckbox } from './selection';

const BAR_CHECKBOX =
  'h-4 w-4 shrink-0 cursor-pointer rounded accent-slate-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-slate-800';

interface SelectBarProps {
  /** Every item the panel shows. */
  total: number;
  /** How many are selected. */
  count: number;
  /** The red confirm shows in place of the slate bar. */
  confirming: boolean;
  /** The delete command is running: both confirm buttons are off. */
  busy: boolean;
  onToggleAll: () => void;
  onDelete: () => void;
  onCancel: () => void;
  onConfirm: () => void;
  onKeep: () => void;
}

/**
 * The select bar, in the Brief bar's slot while selecting: Select all, the
 * count, Delete and Cancel. Delete turns the bar red in place to confirm.
 */
export function SelectBar({
  total,
  count,
  confirming,
  busy,
  onToggleAll,
  onDelete,
  onCancel,
  onConfirm,
  onKeep,
}: SelectBarProps) {
  const selectAll = useRef<HTMLInputElement | null>(null);
  const deleteButton = useRef<HTMLButtonElement>(null);
  const confirmButton = useRef<HTMLButtonElement>(null);
  const wasConfirming = useRef(false);

  // Focus: Select all on entering select mode, the confirm button on Delete,
  // and the Delete button again on Keep.
  useEffect(() => {
    selectAll.current?.focus?.();
  }, []);
  useEffect(() => {
    if (confirming) confirmButton.current?.focus?.();
    else if (wasConfirming.current) deleteButton.current?.focus?.();
    wasConfirming.current = confirming;
  }, [confirming]);

  if (confirming) {
    return (
      <div
        role="alertdialog"
        aria-label="Confirm delete"
        className="flex shrink-0 items-center gap-2 border-b border-red-700 bg-red-600 py-2 pl-3.5 pr-2.5 text-white"
      >
        <Trash2 className="h-4 w-4 shrink-0" aria-hidden="true" />
        <p className="min-w-0 flex-1 text-[13.5px] font-semibold leading-snug">
          {`Delete ${count} ${count === 1 ? 'item' : 'items'} for good?`}
        </p>
        <button
          ref={confirmButton}
          type="button"
          disabled={busy}
          onClick={onConfirm}
          aria-label="Confirm delete selected items"
          className="inline-flex h-8 items-center rounded-full bg-white px-3.5 text-[13px] font-bold text-red-700 shadow-sm hover:bg-red-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-red-600 disabled:cursor-not-allowed disabled:opacity-70"
        >
          {`Delete ${count}`}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onKeep}
          aria-label="Keep selected items"
          className="inline-flex h-8 items-center rounded-full px-3 text-[13px] font-semibold text-white/90 hover:bg-white/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white disabled:cursor-not-allowed disabled:opacity-70"
        >
          Keep
        </button>
      </div>
    );
  }

  const all = total > 0 && count === total;
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-slate-900 bg-slate-800 py-2 pl-3.5 pr-2.5 text-white">
      <label className="inline-flex cursor-pointer items-center gap-2 rounded-full py-1 pr-2 text-[13px] font-semibold">
        <TriCheckbox
          inputRef={selectAll}
          checked={all}
          indeterminate={count > 0 && !all}
          disabled={total === 0}
          onChange={onToggleAll}
          label="Select all items"
          className={BAR_CHECKBOX}
        />
        Select all
      </label>
      <span aria-live="polite" className="min-w-0 flex-1 text-[13px] tabular-nums text-slate-300">
        {count === 0 ? 'Pick items to delete' : `${count} selected`}
      </span>
      <button
        ref={deleteButton}
        type="button"
        disabled={count === 0}
        onClick={onDelete}
        aria-label="Delete selected items"
        className="inline-flex h-8 items-center gap-1.5 rounded-full bg-red-500 px-3.5 text-[13px] font-bold text-white hover:bg-red-600 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-slate-800 disabled:cursor-not-allowed disabled:bg-slate-700 disabled:text-slate-400"
      >
        <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
        {count === 0 ? 'Delete' : `Delete ${count}`}
      </button>
      <button
        type="button"
        onClick={onCancel}
        aria-label="Leave select mode"
        className="inline-flex h-8 items-center rounded-full px-3 text-[13px] font-semibold text-slate-200 hover:bg-white/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white"
      >
        Cancel
      </button>
    </div>
  );
}
