'use client';

import { createContext, useContext, useEffect, useRef, type MutableRefObject } from 'react';
import { cn } from '@/lib/utils';

/**
 * Select mode of the live panel. The panel owns the state; the lanes, cards,
 * rows and to-do drawer read it from here. With no provider (a lane rendered
 * on its own) nothing is selecting.
 */
export interface SelectionValue {
  selecting: boolean;
  isSelected: (id: string) => boolean;
  toggle: (id: string) => void;
  /** Select or clear every id in the list. */
  setMany: (ids: string[], on: boolean) => void;
}

const NOT_SELECTING: SelectionValue = {
  selecting: false,
  isSelected: () => false,
  toggle: () => {},
  setMany: () => {},
};

export const SelectionContext = createContext<SelectionValue>(NOT_SELECTING);

export const useSelection = () => useContext(SelectionContext);

export const ITEM_CHECKBOX =
  'h-4 w-4 shrink-0 cursor-pointer rounded border-gray-300 accent-slate-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-500 focus-visible:ring-offset-1';

export const SELECTED_CARD = 'ring-2 ring-slate-700 ring-offset-1 ring-offset-gray-50';
export const SELECTED_ROW = 'rounded-lg border-transparent bg-slate-100 ring-1 ring-inset ring-slate-300';

/** A checkbox that can show the in-between state, which only a DOM property sets. */
export function TriCheckbox({
  checked,
  indeterminate = false,
  onChange,
  label,
  className,
  disabled,
  inputRef,
}: {
  checked: boolean;
  indeterminate?: boolean;
  onChange: () => void;
  label: string;
  className?: string;
  disabled?: boolean;
  /** For a caller that moves focus to the box. */
  inputRef?: MutableRefObject<HTMLInputElement | null>;
}) {
  const ref = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = indeterminate;
  }, [indeterminate]);
  return (
    <input
      ref={(el) => {
        ref.current = el;
        if (inputRef) inputRef.current = el;
      }}
      type="checkbox"
      checked={checked}
      disabled={disabled}
      aria-label={label}
      onChange={onChange}
      // The row around it toggles on click; this click must not toggle twice.
      onClick={(e) => e.stopPropagation()}
      className={className ?? ITEM_CHECKBOX}
    />
  );
}

/** The checkbox of one item. */
export function ItemCheckbox({ id, label, className }: { id: string; label: string; className?: string }) {
  const { isSelected, toggle } = useSelection();
  return <TriCheckbox checked={isSelected(id)} onChange={() => toggle(id)} label={label} className={className} />;
}

/** How many of the ids are selected, and whether that is all of them. */
export function useGroupState(ids: string[]) {
  const { isSelected } = useSelection();
  const count = ids.filter(isSelected).length;
  const all = ids.length > 0 && count === ids.length;
  return { count, all, some: count > 0 && !all };
}

/** The lane header's All checkbox. */
export function LaneAll({ ids, label }: { ids: string[]; label: string }) {
  const { setMany } = useSelection();
  const { all, some } = useGroupState(ids);
  return (
    <label className="inline-flex cursor-pointer items-center gap-1.5 rounded-full py-0.5 pl-1 pr-2 text-[12.5px] font-semibold text-gray-700 hover:bg-gray-100">
      <TriCheckbox
        checked={all}
        indeterminate={some}
        disabled={ids.length === 0}
        onChange={() => setMany(ids, !all)}
        label={label}
      />
      All
    </label>
  );
}

/** A row in select mode: the whole row toggles, and it always carries the room a selected row needs. */
export function useRowSelection(id: string) {
  const { selecting, isSelected, toggle } = useSelection();
  return {
    selecting,
    rowProps: selecting ? { onClick: () => toggle(id) } : {},
    rowClass: cn('-mx-1.5 px-1.5', selecting && 'cursor-pointer', selecting && isSelected(id) && SELECTED_ROW),
  };
}
