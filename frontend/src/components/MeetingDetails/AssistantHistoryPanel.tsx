'use client';

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { BookMarked, ClipboardList, FileText, ListChecks, Trash2 } from 'lucide-react';
import type { MeetingAssistantItem, MeetingAssistantKind, MeetingAssistantMeta } from '@/types';
import { cn } from '@/lib/utils';
import { meetingAssistantService } from '@/services/meetingAssistantService';
import { formatRecordingTime } from '@/components/VirtualizedTranscriptView';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';

const KIND_LABEL: Record<MeetingAssistantKind, string> = {
  answer: 'Answer',
  ask: 'Your question',
  explain: 'Explain',
  catchup: 'Catch-up',
  question: 'Question to ask',
  for_you: 'For you',
  todo: 'To do',
};

// Same hues as the live panel: blue to say, violet to ask, slate for Joseph's own chat.
const KIND_TONE: Record<MeetingAssistantKind, string> = {
  answer: 'bg-blue-50 text-blue-700 ring-blue-200',
  ask: 'bg-slate-100 text-slate-700 ring-slate-200',
  explain: 'bg-slate-100 text-slate-700 ring-slate-200',
  catchup: 'bg-slate-100 text-slate-700 ring-slate-200',
  question: 'bg-violet-50 text-violet-700 ring-violet-200',
  for_you: 'bg-violet-50 text-violet-700 ring-violet-200',
  todo: 'bg-rose-50 text-rose-700 ring-rose-200',
};

const ANSWERED_HOW: Record<string, string> = {
  call: 'on the call',
  chat: 'in Ask',
  you: 'by you',
};

const EMPTY_META: MeetingAssistantMeta = { brief: '', vaultProject: null, vaultNotePath: null, filedAt: null };

const CHECKBOX =
  'h-4 w-4 shrink-0 cursor-pointer rounded border-gray-300 accent-slate-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-500 focus-visible:ring-offset-1';

/** The select-mode checkbox of one item. A click on the row toggles too, so the box stops its own click. */
function SelectBox({ label, checked, onToggle }: { label: string; checked: boolean; onToggle: () => void }) {
  return (
    <input
      type="checkbox"
      aria-label={label}
      checked={checked}
      onChange={onToggle}
      onClick={(event) => event.stopPropagation()}
      className={CHECKBOX}
    />
  );
}

interface Selection {
  /** Select mode is on: checkboxes show and a click on an item toggles it. */
  active: boolean;
  selected: ReadonlySet<string>;
  toggle: (id: string) => void;
}

/** The file name of a vault note path, for either slash. */
function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

function Item({ item, selection }: { item: MeetingAssistantItem; selection: Selection }) {
  const picked = selection.selected.has(item.id);
  const isGap = item.kind === 'question' || item.kind === 'for_you';
  const how = ANSWERED_HOW[item.answeredHow];
  const answered = isGap && item.answer !== '' && how !== undefined;
  return (
    <li>
      <article
        onClick={selection.active ? () => selection.toggle(item.id) : undefined}
        className={cn(
          'rounded-xl border border-gray-200 bg-white px-3.5 py-3',
          picked && 'ring-2 ring-slate-700 ring-offset-1 ring-offset-gray-50',
          selection.active && 'cursor-pointer'
        )}
      >
        <div className="mb-1.5 flex items-center gap-2">
          {selection.active && (
            <SelectBox
              label={`Select item: ${item.lead || item.question}`}
              checked={picked}
              onToggle={() => selection.toggle(item.id)}
            />
          )}
          <span className="shrink-0 text-xs tabular-nums text-gray-600">{formatRecordingTime(item.callTime)}</span>
          <span
            className={cn(
              'inline-flex h-5 shrink-0 items-center whitespace-nowrap rounded-full px-2 text-xs font-semibold leading-none ring-1 ring-inset',
              KIND_TONE[item.kind]
            )}
          >
            {KIND_LABEL[item.kind]}
          </span>
        </div>
        {item.question && (
          <p className="mb-1.5 break-words text-[13.5px] italic text-gray-600">&ldquo;{item.question}&rdquo;</p>
        )}
        {item.lead && (
          <p className="break-words text-[15px] font-semibold leading-snug text-gray-900">{item.lead}</p>
        )}
        {item.bullets.length > 0 && (
          <ul className="mt-1.5 grid list-none gap-1 p-0">
            {item.bullets.map((bullet, i) => (
              <li
                key={i}
                className='relative rounded pl-3.5 text-sm leading-snug text-gray-700 before:absolute before:left-0.5 before:top-[0.62em] before:h-0.5 before:w-1.5 before:rounded-sm before:bg-gray-300 before:content-[""]'
              >
                {bullet}
              </li>
            ))}
          </ul>
        )}
        {isGap && item.why && <p className="mt-1.5 break-words text-sm text-gray-600">{item.why}</p>}
        {answered && (
          <p className="mt-2 break-words rounded-lg bg-emerald-50 px-2.5 py-1.5 text-sm text-emerald-800">
            Answered ({how}){item.answeredCallTime !== null && <> at {formatRecordingTime(item.answeredCallTime)}</>}:{' '}
            {item.answer}
          </p>
        )}
        {item.source && <p className="mt-2 break-words font-mono text-[11px] text-gray-600">{item.source}</p>}
      </article>
    </li>
  );
}

const CHIP_BASE =
  'inline-flex h-5 shrink-0 items-center gap-1 whitespace-nowrap rounded-full text-xs font-semibold leading-none ring-1 ring-inset';

function OwnerPill({ owner }: { owner: 'you' | 'them' }) {
  return owner === 'you' ? (
    <span className="inline-flex h-[18px] shrink-0 items-center justify-center rounded-full bg-rose-600 px-2 text-[11px] font-bold leading-none text-white">
      You
    </span>
  ) : (
    <span className="inline-flex h-[18px] shrink-0 items-center justify-center rounded-full bg-white px-2 text-[11px] font-bold leading-none text-gray-700 ring-1 ring-inset ring-gray-300">
      Them
    </span>
  );
}

function TodoColumn({
  owner,
  title,
  todos,
  className,
  selection,
}: {
  owner: 'you' | 'them';
  title: string;
  todos: MeetingAssistantItem[];
  className?: string;
  selection: Selection;
}) {
  // One to-do row: a checkbox slot before the text while selecting, the selected look when ticked.
  const row = (t: MeetingAssistantItem, body: ReactNode) => {
    const picked = selection.selected.has(t.id);
    return (
      <li
        key={t.id}
        onClick={selection.active ? () => selection.toggle(t.id) : undefined}
        className={cn(
          '-mx-1.5 flex items-start gap-2 border-t border-gray-100 px-1.5 py-1.5 first:border-t-0',
          picked && 'rounded-lg border-transparent bg-slate-100 ring-1 ring-inset ring-slate-300',
          selection.active && 'cursor-pointer'
        )}
      >
        {selection.active && (
          <span className="mt-0.5 grid h-5 w-4 shrink-0 place-items-center">
            <SelectBox label={`Select to-do: ${t.lead}`} checked={picked} onToggle={() => selection.toggle(t.id)} />
          </span>
        )}
        <div className="min-w-0 flex-1">{body}</div>
      </li>
    );
  };
  const open = todos.filter((t) => t.answeredHow === '');
  const done = todos.filter((t) => t.answeredHow !== '');
  return (
    <div className={cn('min-w-0 px-3 py-2.5', className)}>
      <div className="flex items-center gap-2 pb-1">
        <OwnerPill owner={owner} />
        <h3 className="text-xs font-semibold text-gray-700">{title}</h3>
      </div>
      {todos.length === 0 && (
        <p className="py-1.5 text-xs text-gray-600">{owner === 'you' ? 'Nothing promised' : 'Nothing owed'}</p>
      )}
      <ul className="flex list-none flex-col p-0">
        {open.map((t) =>
          row(
            t,
            <>
              <p className="break-words text-sm font-medium leading-snug text-gray-900">{t.lead}</p>
              <p className="mt-0.5 text-xs tabular-nums text-gray-600">
                {owner === 'them' && t.ownerName && <>{t.ownerName} &middot; </>}
                {t.due && (
                  <>
                    <b className="font-semibold text-gray-800">{t.due}</b> &middot;{' '}
                  </>
                )}
                {formatRecordingTime(t.callTime)}
              </p>
            </>
          )
        )}
      </ul>
      {done.length > 0 && (
        <>
          <div className="flex items-center gap-2 pb-1 pt-3">
            <h4 className="text-xs font-semibold text-gray-700">Done</h4>
            <span className="text-xs tabular-nums text-gray-600">{done.length}</span>
          </div>
          <ul className="flex list-none flex-col p-0">
            {done.map((t) =>
              row(
                t,
                <>
                  <p className="break-words text-sm leading-snug text-gray-600">{t.lead}</p>
                  <p className="mt-0.5 text-xs tabular-nums text-gray-600">
                    Done{t.answeredCallTime !== null && <> {formatRecordingTime(t.answeredCallTime)}</>}
                    {t.answer && <> &middot; {t.answer}</>}
                  </p>
                </>
              )
            )}
          </ul>
        </>
      )}
    </div>
  );
}

/** The saved to-dos of a call: what Joseph promised and what the other side owes him. */
function TodoLedger({ todos, selection }: { todos: MeetingAssistantItem[]; selection: Selection }) {
  return (
    <section aria-label="To do" className="overflow-hidden rounded-xl border border-rose-200 bg-white">
      <div className="flex items-center gap-2 border-b border-rose-100 bg-rose-50 px-3 py-2">
        <span className={cn(CHIP_BASE, 'bg-rose-50 pl-1.5 pr-2 text-rose-700 ring-rose-200')}>
          <ClipboardList aria-hidden="true" className="h-3 w-3" />
          To do
        </span>
        <span className="text-xs text-rose-900">{todos.length} from this call. Kept here only, never sent.</span>
      </div>
      <div className="grid grid-cols-2">
        <TodoColumn owner="you" title="You promised" todos={todos.filter((t) => t.owner === 'you')} selection={selection} />
        <TodoColumn
          owner="them"
          title="They owe you"
          todos={todos.filter((t) => t.owner !== 'you')}
          className="border-l border-gray-200"
          selection={selection}
        />
      </div>
    </section>
  );
}


type LoadState = 'loading' | 'ready' | 'failed';

const sortItems = (loaded: MeetingAssistantItem[]) => [...loaded].sort((a, b) => a.callTime - b.callTime);

/** The meta of an older build that has no such command, or of a meeting with no record, is empty. */
const loadMeta = (meetingId: string) =>
  meetingAssistantService.getMeta(meetingId).catch((error): MeetingAssistantMeta => {
    console.error('Failed to load the assistant meta:', error);
    return EMPTY_META;
  });

/** What the live assistant showed during one past call, in call-time order, with a way to select and delete it. */
export function AssistantHistoryPanel({ meetingId }: { meetingId: string }) {
  const [items, setItems] = useState<MeetingAssistantItem[]>([]);
  const [meta, setMeta] = useState<MeetingAssistantMeta>(EMPTY_META);
  const [state, setState] = useState<LoadState>('loading');
  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteFailed, setDeleteFailed] = useState(false);
  const selectButtonRef = useRef<HTMLButtonElement>(null);
  const selectAllRef = useRef<HTMLInputElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const focusSelectOnLeave = useRef(false);

  useEffect(() => {
    let current = true;
    setItems([]);
    setMeta(EMPTY_META);
    setState('loading');
    setSelecting(false);
    setSelected(new Set());
    setConfirming(false);
    setDeleteFailed(false);
    Promise.all([meetingAssistantService.getItems(meetingId), loadMeta(meetingId)])
      .then(([loaded, loadedMeta]) => {
        if (!current) return;
        setItems(sortItems(loaded));
        setMeta(loadedMeta);
        setState('ready');
      })
      .catch((error) => {
        console.error('Failed to load the assistant items:', error);
        if (current) setState('failed');
      });
    return () => {
      current = false;
    };
  }, [meetingId]);

  const total = items.length;
  const count = selected.size;
  const everything = total > 0 && count === total;

  const toggle = useCallback((id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }, []);

  const startSelecting = () => {
    setSelected(new Set());
    setDeleteFailed(false);
    setSelecting(true);
  };

  const leaveSelecting = useCallback(() => {
    focusSelectOnLeave.current = true;
    setSelecting(false);
    setSelected(new Set());
  }, []);

  const toggleAll = () => setSelected(everything ? new Set() : new Set(items.map((item) => item.id)));

  // Focus goes to Select all on entering select mode and back to Select on leaving it.
  useEffect(() => {
    if (selecting) {
      selectAllRef.current?.focus();
    } else if (focusSelectOnLeave.current) {
      focusSelectOnLeave.current = false;
      (selectButtonRef.current ?? headingRef.current)?.focus();
    }
  }, [selecting]);

  // Some but not all selected: the Select all box shows the dash.
  useEffect(() => {
    if (selectAllRef.current) selectAllRef.current.indeterminate = count > 0 && !everything;
  }, [selecting, count, everything]);

  // Esc leaves select mode; with the dialog open, Radix handles Esc and marks the event.
  useEffect(() => {
    if (!selecting || confirming) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !event.defaultPrevented) leaveSelecting();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [selecting, confirming, leaveSelecting]);

  const confirmDelete = useCallback(async () => {
    const ids = items.filter((item) => selected.has(item.id)).map((item) => item.id);
    setDeleting(true);
    try {
      await meetingAssistantService.deleteSelected(meetingId, ids);
      // The backend rewrote assistant.json and, when filed, the vault note: show what it kept.
      try {
        const [loaded, loadedMeta] = await Promise.all([meetingAssistantService.getItems(meetingId), loadMeta(meetingId)]);
        setItems(sortItems(loaded));
        setMeta(loadedMeta);
      } catch (error) {
        console.error('Failed to reload the assistant items:', error);
        setItems((prev) => prev.filter((item) => !ids.includes(item.id)));
      }
      setSelecting(false);
      setSelected(new Set());
      setConfirming(false);
      setDeleteFailed(false);
    } catch (error) {
      console.error('Failed to delete the assistant items:', error);
      setConfirming(false);
      setDeleteFailed(true);
    } finally {
      setDeleting(false);
    }
  }, [items, selected, meetingId]);

  const selection: Selection = { active: selecting, selected, toggle };
  const todos = items.filter((item) => item.kind === 'todo');
  const others = items.filter((item) => item.kind !== 'todo');
  const vaultFile = meta.vaultNotePath ? baseName(meta.vaultNotePath) : null;
  const briefText = meta.brief.trim();
  // The dialog keeps the numbers it opened with while it fades out after the delete emptied the selection.
  const asked = useRef({ everything, total, count });
  if (confirming) asked.current = { everything, total, count };
  const countText = count === 0 ? 'Pick items to delete' : everything ? `All ${total} selected` : `${count} of ${total} selected`;

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-white">
      {selecting ? (
        <div
          role="toolbar"
          aria-label="Assistant selection"
          className="flex shrink-0 items-center gap-3 border-b border-slate-900 bg-slate-800 py-2 pl-4 pr-3 text-white"
        >
          <label className="inline-flex cursor-pointer items-center gap-2 text-[13.5px] font-semibold">
            <input
              ref={selectAllRef}
              type="checkbox"
              aria-label="Select all assistant items"
              checked={everything}
              onChange={toggleAll}
              className="h-4 w-4 shrink-0 cursor-pointer rounded accent-slate-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-slate-800"
            />
            Select all
          </label>
          <span className="min-w-0 flex-1 text-[13.5px] tabular-nums text-slate-300">{countText}</span>
          <button
            type="button"
            aria-label="Delete selected assistant items"
            disabled={count === 0}
            onClick={() => setConfirming(true)}
            className="inline-flex h-8 items-center gap-1.5 rounded-full bg-red-500 px-3.5 text-[13px] font-bold text-white hover:bg-red-600 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-slate-800 disabled:cursor-not-allowed disabled:bg-slate-700 disabled:text-slate-400"
          >
            <Trash2 aria-hidden="true" className="h-3.5 w-3.5" />
            {count > 0 ? `Delete ${count}` : 'Delete'}
          </button>
          <button
            type="button"
            aria-label="Leave assistant select mode"
            onClick={leaveSelecting}
            className="inline-flex h-8 items-center rounded-full px-3 text-[13px] font-semibold text-slate-200 hover:bg-white/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white"
          >
            Cancel
          </button>
        </div>
      ) : (
        <div className="flex shrink-0 items-center justify-between gap-3 border-b border-gray-200 px-4 py-3">
          <h2 ref={headingRef} tabIndex={-1} className="text-sm font-semibold text-gray-900 focus-visible:outline-none">
            Assistant{state === 'ready' && total > 0 && <span className="ml-2 font-normal text-gray-600">{total}</span>}
          </h2>
          {state === 'ready' && total > 0 && (
            <button
              ref={selectButtonRef}
              type="button"
              aria-label="Select assistant items"
              onClick={startSelecting}
              className="inline-flex h-8 items-center gap-1.5 rounded-full border border-gray-200 bg-white px-3 text-[13px] font-semibold text-gray-700 hover:bg-gray-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-500"
            >
              <ListChecks aria-hidden="true" className="h-4 w-4" />
              Select
            </button>
          )}
        </div>
      )}
      <div
        role="region"
        aria-label="Assistant items"
        tabIndex={0}
        className="min-h-0 flex-1 overflow-y-auto px-4 py-3 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-500"
      >
        {deleteFailed && (
          <p role="alert" className="mb-3 text-sm text-red-700">Could not delete the selected items.</p>
        )}
        {state === 'loading' && <p className="text-sm text-gray-600">Loading the assistant items.</p>}
        {state === 'failed' && <p className="text-sm text-gray-700">Could not load the assistant items.</p>}
        {state === 'ready' && vaultFile && (
          <p className="mb-2.5 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-[13px] text-gray-600">
            <BookMarked aria-hidden="true" className="h-4 w-4 shrink-0 text-gray-400" />
            Filed to the vault
            {meta.vaultProject && (
              <>
                {' '}under
                <span className="inline-flex h-[22px] items-center rounded-full bg-gray-100 px-2 text-[12.5px] font-semibold text-gray-800 ring-1 ring-inset ring-gray-200">
                  {meta.vaultProject}
                </span>
              </>
            )}
            as
            <span className="break-all font-semibold text-gray-800">{vaultFile}</span>
          </p>
        )}
        {state === 'ready' && briefText !== '' && (
          <section
            aria-label="Brief"
            className="mb-4 flex items-start gap-2.5 rounded-xl border border-gray-200 bg-gray-50 px-3.5 py-3"
          >
            <FileText aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0 text-gray-400" />
            <p className="min-w-0 flex-1 whitespace-pre-line break-words text-sm leading-relaxed text-gray-700">
              <b className="font-semibold text-gray-900">Brief:</b> {briefText}
            </p>
          </section>
        )}
        {state === 'ready' && total === 0 && (
          <p className="text-sm text-gray-600">No assistant items were saved for this meeting.</p>
        )}
        {state === 'ready' && total > 0 && (
          <>
            {todos.length > 0 && <TodoLedger todos={todos} selection={selection} />}
            {todos.length > 0 && others.length > 0 && (
              <div className="mb-2.5 mt-4 flex items-center gap-2 text-xs font-semibold text-gray-600">
                Everything the assistant showed, in call order
                <span aria-hidden="true" className="h-px flex-1 bg-gray-200" />
              </div>
            )}
            {others.length > 0 && (
              <ol className="grid list-none gap-2.5 p-0">
                {others.map((item) => (
                  <Item key={item.id} item={item} selection={selection} />
                ))}
              </ol>
            )}
          </>
        )}
      </div>
      <Dialog open={confirming} onOpenChange={(open) => { if (!deleting) setConfirming(open); }}>
        <DialogContent
          onCloseAutoFocus={(event) => {
            // No trigger element wraps the button, so hand focus back by hand: to Select, or after
            // a delete that left nothing, to the panel heading.
            const target = selectButtonRef.current ?? headingRef.current;
            if (target) {
              event.preventDefault();
              target.focus();
            }
          }}
        >
          <DialogHeader>
            <DialogTitle>{asked.current.everything ? `Delete all ${asked.current.total} assistant items?` : `Delete ${asked.current.count} assistant ${asked.current.count === 1 ? 'item' : 'items'}?`}</DialogTitle>
            <DialogDescription>
              {asked.current.everything
                ? 'They are removed from this meeting, and assistant.json is removed from the recording folder.'
                : 'They are removed from this meeting and from assistant.json.'}
              {vaultFile && (
                <>
                  {' '}The vault note <span className="font-semibold text-gray-800">{vaultFile}</span>
                  {meta.vaultProject && (
                    <>
                      {' '}in <span className="whitespace-nowrap font-semibold text-gray-800">{meta.vaultProject}</span>
                    </>
                  )}{' '}
                  is updated too.
                </>
              )}{' '}
              This can&rsquo;t be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <button
              type="button"
              aria-label="Keep assistant items"
              onClick={() => setConfirming(false)}
              disabled={deleting}
              className="rounded-md border border-gray-300 bg-white px-3 py-1.5 text-sm text-gray-800 hover:bg-gray-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
            >
              Keep
            </button>
            <button
              type="button"
              aria-label="Confirm delete assistant items"
              onClick={() => void confirmDelete()}
              disabled={deleting}
              className="rounded-md bg-red-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-red-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-500 focus-visible:ring-offset-2"
            >
              {asked.current.everything ? 'Delete all' : `Delete ${asked.current.count}`}
            </button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
