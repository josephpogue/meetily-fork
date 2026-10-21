'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { ClipboardList } from 'lucide-react';
import type { MeetingAssistantItem, MeetingAssistantKind } from '@/types';
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

function Item({ item }: { item: MeetingAssistantItem }) {
  const isGap = item.kind === 'question' || item.kind === 'for_you';
  const how = ANSWERED_HOW[item.answeredHow];
  const answered = isGap && item.answer !== '' && how !== undefined;
  return (
    <li>
      <article className="rounded-xl border border-gray-200 bg-white px-3.5 py-3">
        <div className="mb-1.5 flex items-center gap-2">
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
}: {
  owner: 'you' | 'them';
  title: string;
  todos: MeetingAssistantItem[];
  className?: string;
}) {
  const open = todos.filter((t) => t.answeredHow === '');
  const done = todos.filter((t) => t.answeredHow !== '');
  return (
    <div className={cn('min-w-0 px-3 py-2.5', className)}>
      <div className="flex items-center gap-2 pb-1">
        <OwnerPill owner={owner} />
        <h3 className="text-xs font-semibold text-gray-700">{title}</h3>
      </div>
      <ul className="flex list-none flex-col p-0">
        {open.map((t) => (
          <li key={t.id} className="border-t border-gray-100 py-1.5 first:border-t-0">
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
          </li>
        ))}
      </ul>
      {done.length > 0 && (
        <>
          <div className="flex items-center gap-2 pb-1 pt-3">
            <h4 className="text-xs font-semibold text-gray-700">Done</h4>
            <span className="text-xs tabular-nums text-gray-600">{done.length}</span>
          </div>
          <ul className="flex list-none flex-col p-0">
            {done.map((t) => (
              <li key={t.id} className="border-t border-gray-100 py-1.5 first:border-t-0">
                <p className="break-words text-sm leading-snug text-gray-600">{t.lead}</p>
                <p className="mt-0.5 text-xs tabular-nums text-gray-600">
                  Done{t.answeredCallTime !== null && <> {formatRecordingTime(t.answeredCallTime)}</>}
                  {t.answer && <> &middot; {t.answer}</>}
                </p>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

/** The saved to-dos of a call: what Joseph promised and what the other side owes him. */
function TodoLedger({ todos }: { todos: MeetingAssistantItem[] }) {
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
        <TodoColumn owner="you" title="You promised" todos={todos.filter((t) => t.owner === 'you')} />
        <TodoColumn
          owner="them"
          title="They owe you"
          todos={todos.filter((t) => t.owner !== 'you')}
          className="border-l border-gray-200"
        />
      </div>
    </section>
  );
}

type LoadState = 'loading' | 'ready' | 'failed';

/** What the live assistant showed during one past call, in call-time order, with a way to delete it. */
export function AssistantHistoryPanel({ meetingId }: { meetingId: string }) {
  const [items, setItems] = useState<MeetingAssistantItem[]>([]);
  const [state, setState] = useState<LoadState>('loading');
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteFailed, setDeleteFailed] = useState(false);
  const deleteButtonRef = useRef<HTMLButtonElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    let current = true;
    setItems([]);
    setState('loading');
    setConfirming(false);
    setDeleteFailed(false);
    meetingAssistantService
      .getItems(meetingId)
      .then((loaded) => {
        if (!current) return;
        setItems([...loaded].sort((a, b) => a.callTime - b.callTime));
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

  const confirmDelete = useCallback(async () => {
    setDeleting(true);
    try {
      await meetingAssistantService.deleteItems(meetingId);
      setItems([]);
      setConfirming(false);
      setDeleteFailed(false);
    } catch (error) {
      console.error('Failed to delete the assistant items:', error);
      setConfirming(false);
      setDeleteFailed(true);
    } finally {
      setDeleting(false);
    }
  }, [meetingId]);

  const todos = items.filter((item) => item.kind === 'todo');
  const others = items.filter((item) => item.kind !== 'todo');

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-white">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-gray-200 px-4 py-3">
        <h2 ref={headingRef} tabIndex={-1} className="text-sm font-semibold text-gray-900 focus-visible:outline-none">
          Assistant{state === 'ready' && items.length > 0 && <span className="ml-2 font-normal text-gray-600">{items.length}</span>}
        </h2>
        {state === 'ready' && items.length > 0 && (
          <button
            ref={deleteButtonRef}
            type="button"
            onClick={() => setConfirming(true)}
            className="rounded px-2 py-1 text-sm text-red-700 hover:bg-red-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
          >
            Delete assistant items
          </button>
        )}
      </div>
      <div
        role="region"
        aria-label="Assistant items"
        tabIndex={0}
        className="min-h-0 flex-1 overflow-y-auto px-4 py-3 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-500"
      >
        {deleteFailed && (
          <p role="alert" className="mb-3 text-sm text-red-700">Could not delete the assistant items.</p>
        )}
        {state === 'loading' && <p className="text-sm text-gray-600">Loading the assistant items.</p>}
        {state === 'failed' && <p className="text-sm text-gray-700">Could not load the assistant items.</p>}
        {state === 'ready' && items.length === 0 && (
          <p className="text-sm text-gray-600">No assistant items were saved for this meeting.</p>
        )}
        {state === 'ready' && items.length > 0 && (
          <>
            {todos.length > 0 && <TodoLedger todos={todos} />}
            {todos.length > 0 && others.length > 0 && (
              <div className="mb-2.5 mt-4 flex items-center gap-2 text-xs font-semibold text-gray-600">
                Everything the assistant showed, in call order
                <span aria-hidden="true" className="h-px flex-1 bg-gray-200" />
              </div>
            )}
            {others.length > 0 && (
              <ol className="grid list-none gap-2.5 p-0">
                {others.map((item) => (
                  <Item key={item.id} item={item} />
                ))}
              </ol>
            )}
          </>
        )}
      </div>
      <Dialog open={confirming} onOpenChange={(open) => { if (!deleting) setConfirming(open); }}>
        <DialogContent
          onCloseAutoFocus={(event) => {
            // No trigger element wraps the button, so hand focus back to it by hand. After a
            // delete the button is gone, so focus lands on the panel heading instead.
            const target = deleteButtonRef.current ?? headingRef.current;
            if (target) {
              event.preventDefault();
              target.focus();
            }
          }}
        >
          <DialogHeader>
            <DialogTitle>Delete assistant items</DialogTitle>
            <DialogDescription>
              Delete the saved assistant items for this meeting? This also removes assistant.json from the recording folder.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <button
              type="button"
              onClick={() => setConfirming(false)}
              disabled={deleting}
              className="rounded-md border border-gray-300 bg-white px-3 py-1.5 text-sm text-gray-800 hover:bg-gray-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => void confirmDelete()}
              disabled={deleting}
              className="rounded-md bg-red-600 px-3 py-1.5 text-sm text-white hover:bg-red-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-500 focus-visible:ring-offset-2"
            >
              Delete
            </button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
