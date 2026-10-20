'use client';

import { KeyboardEvent, useEffect, useState } from 'react';
import { Check, CircleHelp, Loader2, RefreshCw, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useAssistant } from '@/contexts/AssistantContext';
import { AssistantAskItem } from '@/types';
import { answeredHowLine, appendBriefAnswer, formatOpenStamp, newAskIds } from '@/lib/ask-next';
import { cn } from '@/lib/utils';
import { KindChip, LANE_TONE } from './KindChip';

// How often the "Checked 12s ago" stamp re-renders.
const STAMP_TICK_MS = 5000;
// How long the items a deep check added or reworded stay highlighted.
const CHANGED_HIGHLIGHT_MS = 2500;

const DISMISS_CLASS =
  'mt-0.5 shrink-0 rounded text-gray-300 opacity-0 hover:text-gray-500 focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400 group-hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-50';

/**
 * The Ask lane: open gap questions, For-you questions, then every answered
 * question with its answer and how it got answered. The lane scrolls as a
 * whole; nothing in it is clamped. Refreshes when someone finishes speaking,
 * on the Refresh button, and on Option-Q. The last lists stay after the call.
 */
export function AskLane({ disabled }: { disabled: boolean }) {
  const { status, asks, refreshAsks, dismissAsk, answerAsk, setBrief } = useAssistant();
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), STAMP_TICK_MS);
    return () => clearInterval(timer);
  }, []);

  // A new deep result lights its changed items for a moment. Keyed on the
  // result (updatedAt moves only when a list is applied), so a snapshot that
  // repeats the same changed ids, such as a brief edit, does not relight them.
  const [highlightChanged, setHighlightChanged] = useState(false);
  const changedKey = asks.changed.length > 0 ? `${asks.updatedAt}:${asks.changed.join(',')}` : '';
  useEffect(() => {
    if (!changedKey) {
      setHighlightChanged(false);
      return;
    }
    setHighlightChanged(true);
    const timer = setTimeout(() => setHighlightChanged(false), CHANGED_HIGHLIGHT_MS);
    return () => clearTimeout(timer);
  }, [changedKey]);
  const isChanged = (id: string) => highlightChanged && asks.changed.includes(id);

  // "New" marks an open item that was not on the list before this one. The
  // first list the lane sees has nothing before it, so nothing is new there.
  const openIds = asks.asks.map((a) => a.id);
  const idsKey = openIds.join(',');
  const hasItems = asks.asks.length > 0 || asks.forYou.length > 0;
  const [track, setTrack] = useState<{ key: string; ids: string[] | null; fresh: string[] }>({
    key: '',
    ids: null,
    fresh: [],
  });
  if (track.key !== idsKey && (track.ids !== null || hasItems || asks.updatedAt !== null)) {
    setTrack({ key: idsKey, ids: openIds, fresh: newAskIds(track.ids, openIds) });
  }

  const openCount = asks.asks.length + asks.forYou.length;
  const hasAnswered = asks.answered.length > 0;
  if (!status.sessionOpen && !hasItems && !hasAnswered) {
    return (
      <section aria-label="Ask" className="flex min-h-0 min-w-0 flex-1 flex-col">
        <LaneHeader openCount={0} answeredCount={0} thinking={false} refreshOff onRefresh={refreshAsks} />
        <p className="px-3 py-2 text-xs text-gray-400">Questions appear once the call starts.</p>
      </section>
    );
  }

  const thinking = asks.state === 'thinking';
  const failed = asks.state === 'failed';

  const answerForYou = async (item: AssistantAskItem, answer: string) => {
    await setBrief(appendBriefAnswer(asks.brief, item.question, answer));
    await answerAsk(item.id, answer);
  };

  let emptyMessage: string | null = null;
  if (asks.state === 'unavailable') {
    emptyMessage = 'Questions unavailable for this call.';
  } else if (!hasItems && asks.updatedAt === null) {
    emptyMessage = 'Listening. Questions appear after the first exchange.';
  } else if (!hasItems) {
    emptyMessage = 'Nothing to ask right now.';
  }

  const stamp = failed ? 'Refresh failed' : formatOpenStamp(asks.updatedAt, now, asks.checked);

  return (
    <section aria-label="Ask" className="flex min-h-0 min-w-0 flex-1 flex-col">
      <LaneHeader
        openCount={openCount}
        answeredCount={asks.answered.length}
        thinking={thinking}
        refreshOff={disabled || asks.state === 'unavailable'}
        onRefresh={refreshAsks}
      />
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
        <div className="flex items-center justify-between gap-2 py-2">
          <h3 className="flex items-center gap-1.5 text-xs font-semibold text-gray-700">
            <KindChip kind="toAsk" />
            Open
          </h3>
          <span className="flex items-center gap-1 text-xs text-gray-400">
            {asks.checking ? (
              <>
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-violet-400" />
                Checking
              </>
            ) : (
              <span className={failed ? 'text-gray-500' : undefined}>{stamp}</span>
            )}
          </span>
        </div>

        {emptyMessage ? (
          <p className="text-xs text-gray-400">{emptyMessage}</p>
        ) : (
          <ol className="flex flex-col">
            {asks.asks.map((item, i) => (
              <li key={item.id} className="group flex items-start gap-2.5 border-t border-gray-100 py-2 first:border-t-0">
                <span
                  className={cn(
                    'mt-px flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[11px] font-bold ring-1 ring-inset',
                    'bg-violet-50 text-violet-700 ring-violet-200'
                  )}
                >
                  {i + 1}
                </span>
                <div
                  className={cn(
                    'min-w-0 flex-1 rounded transition-colors duration-1000',
                    isChanged(item.id) && 'bg-violet-50'
                  )}
                >
                  <p className="break-words text-sm font-medium leading-snug text-gray-900">
                    {item.question}
                    {track.fresh.includes(item.id) && (
                      <span className="ml-1 text-[11px] font-bold text-violet-700">New</span>
                    )}
                  </p>
                  {item.why && <p className="mt-0.5 break-words text-xs leading-snug text-gray-500">{item.why}</p>}
                </div>
                <DismissButton question={item.question} disabled={disabled} onClick={() => dismissAsk(item.id)} />
              </li>
            ))}
          </ol>
        )}

        {asks.forYou.length > 0 && (
          <div className="mt-2 rounded-lg border border-gray-200 bg-gray-50 px-3 py-2.5">
            <p className="mb-1 text-xs font-semibold text-gray-600">For you</p>
            <ul className="flex flex-col gap-3">
              {asks.forYou.map((item) => (
                <ForYouItem
                  key={item.id}
                  item={item}
                  changed={isChanged(item.id)}
                  disabled={disabled}
                  onAnswer={(answer) => answerForYou(item, answer)}
                  onDismiss={() => dismissAsk(item.id)}
                />
              ))}
            </ul>
          </div>
        )}

        <div className="flex items-center justify-between gap-2 pb-1 pt-5">
          <h3 className="flex items-center gap-1.5 text-xs font-semibold text-gray-700">
            <KindChip kind="answered" />
            With answers
          </h3>
          <span className="text-xs text-gray-400">Newest first</span>
        </div>
        {hasAnswered ? (
          <ul className="flex flex-col">
            {asks.answered.map((item) => (
              <li key={item.id} className="flex items-start gap-2.5 border-t border-gray-100 py-2 first:border-t-0">
                <span
                  className={cn(
                    'mt-px flex h-5 w-5 shrink-0 items-center justify-center rounded-full ring-1 ring-inset',
                    'bg-emerald-50 text-emerald-700 ring-emerald-200'
                  )}
                >
                  <Check className="h-3 w-3" aria-hidden="true" />
                </span>
                <div className="min-w-0 flex-1">
                  <p className="break-words text-[13px] font-semibold leading-snug text-gray-500">{item.question}</p>
                  <p className="mt-0.5 break-words text-sm leading-snug text-gray-900">{item.answer}</p>
                  <p className="mt-0.5 text-xs text-gray-500">{answeredHowLine(item.how, item.at)}</p>
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-xs text-gray-400">Answered questions land here with their answers.</p>
        )}
      </div>
    </section>
  );
}

function LaneHeader({
  openCount,
  answeredCount,
  thinking,
  refreshOff,
  onRefresh,
}: {
  openCount: number;
  answeredCount: number;
  thinking: boolean;
  refreshOff: boolean;
  onRefresh: () => void;
}) {
  return (
    <header className="flex shrink-0 items-center gap-2 border-b border-gray-100 px-3 py-2">
      <span className={cn('flex h-6 w-6 items-center justify-center rounded-md', LANE_TONE.ask.badge)}>
        <CircleHelp className="h-3.5 w-3.5" aria-hidden="true" />
      </span>
      <h2 className={cn('text-sm font-semibold', LANE_TONE.ask.title)}>Ask</h2>
      <span className="flex-1 text-xs tabular-nums text-gray-500">
        {`${openCount} open, ${answeredCount} answered`}
      </span>
      <Button
        variant="ghost"
        size="icon"
        className="h-6 w-6"
        onClick={onRefresh}
        disabled={refreshOff}
        title="Refresh (Option-Q)"
        aria-label="Refresh questions"
      >
        {thinking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
      </Button>
    </header>
  );
}

function DismissButton({ question, disabled, onClick }: { question: string; disabled: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title="Dismiss"
      aria-label={`Dismiss ${question}`}
      disabled={disabled}
      className={DISMISS_CLASS}
    >
      <X className="h-3.5 w-3.5" />
    </button>
  );
}

function ForYouItem({
  item,
  changed,
  disabled,
  onAnswer,
  onDismiss,
}: {
  item: AssistantAskItem;
  changed: boolean;
  disabled: boolean;
  onAnswer: (answer: string) => void;
  onDismiss: () => void;
}) {
  const [value, setValue] = useState('');

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' && value.trim()) {
      e.preventDefault();
      onAnswer(value);
      setValue('');
    }
  };

  return (
    <li className="group flex flex-col gap-1.5">
      <div className="flex items-start gap-2">
        <p
          className={cn(
            'min-w-0 flex-1 break-words rounded text-sm text-gray-900 transition-colors duration-1000',
            changed && 'bg-violet-50'
          )}
        >
          {item.question}
        </p>
        <DismissButton question={item.question} disabled={disabled} onClick={onDismiss} />
      </div>
      <Input
        aria-label={`Answer: ${item.question}`}
        disabled={disabled}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Answer, then Enter"
        className="h-7 bg-white text-xs"
      />
    </li>
  );
}
