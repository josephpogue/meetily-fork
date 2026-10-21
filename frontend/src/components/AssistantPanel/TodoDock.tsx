'use client';

import { useEffect, useRef, useState } from 'react';
import { useAssistant } from '@/contexts/AssistantContext';
import { TodoStrip } from './TodoStrip';
import { TodoDrawerColumns } from './TodoDrawer';

/**
 * The To do strip with its drawer, placed at the bottom of the lanes body (the
 * nearest positioned ancestor). Closed, the strip is one line in the flow; open,
 * a drawer covers the lower part of both lanes and the strip underneath is kept
 * for its height only, so the lanes never resize. Esc closes the drawer and
 * focus returns to the strip's button. "New" means arrived since the drawer was
 * last closed.
 */
export function TodoDock() {
  const { asks, dismissTodo } = useAssistant();
  const todos = asks.todos ?? [];
  const [open, setOpen] = useState(false);
  const [lastClosedAt, setLastClosedAt] = useState(0);
  const openButton = useRef<HTMLButtonElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const wasOpen = useRef(false);

  const toggle = () => {
    if (open) setLastClosedAt(Date.now());
    setOpen(!open);
  };

  useEffect(() => {
    if (open && !wasOpen.current) closeButton.current?.focus?.();
    if (!open && wasOpen.current) openButton.current?.focus?.();
    wasOpen.current = open;
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      e.preventDefault();
      setLastClosedAt(Date.now());
      setOpen(false);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open]);

  return (
    <>
      <TodoStrip
        todos={todos}
        lastClosedAt={lastClosedAt}
        open={false}
        onToggle={toggle}
        buttonRef={openButton}
        covered={open}
      />
      {open && (
        <div
          role="region"
          aria-label="To do list"
          className="absolute inset-x-0 bottom-0 z-10 flex h-[60%] flex-col overflow-hidden rounded-t-xl border-t border-rose-200 bg-white shadow-[0_-10px_28px_-14px_rgba(76,5,25,0.22)] duration-200 animate-in slide-in-from-bottom-4 motion-reduce:animate-none"
        >
          <TodoStrip todos={todos} lastClosedAt={lastClosedAt} open onToggle={toggle} buttonRef={closeButton} />
          <TodoDrawerColumns todos={todos} lastClosedAt={lastClosedAt} onDismiss={(id) => void dismissTodo(id)} />
        </div>
      )}
    </>
  );
}
