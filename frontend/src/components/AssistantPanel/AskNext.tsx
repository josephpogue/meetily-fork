'use client';

import { KeyboardEvent, useEffect, useState } from 'react';
import { Loader2, RefreshCw, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useAssistant } from '@/contexts/AssistantContext';
import { AssistantAskItem } from '@/types';
import { appendBriefAnswer, formatUpdatedAgo } from '@/lib/ask-next';
import { cn } from '@/lib/utils';

// How often the "updated 12s ago" stamp re-renders.
const STAMP_TICK_MS = 5000;
// How long the items a deep check added or reworded stay highlighted, as on
// a corrected answer card.
const CHANGED_HIGHLIGHT_MS = 2500;

interface AskNextProps {
  disabled: boolean;
}

/**
 * Questions for Joseph to ask the other side, most important first, plus
 * up to two For-you questions about his own goal. Refreshes when someone
 * finishes speaking, on the Refresh button, and on Option-Q. Hidden until
 * a session opens; the last list stays visible after the call ends.
 */
export function AskNext({ disabled }: AskNextProps) {
  const { status, asks, refreshAsks, dismissAsk, setBrief } = useAssistant();
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

  const hasItems = asks.asks.length > 0 || asks.forYou.length > 0;
  if (!status.sessionOpen && !hasItems) return null;

  const thinking = asks.state === 'thinking';
  const failed = asks.state === 'failed';

  const answerForYou = async (item: AssistantAskItem, answer: string) => {
    await setBrief(appendBriefAnswer(asks.brief, item.question, answer));
    await dismissAsk(item.id);
  };

  let emptyMessage: string | null = null;
  if (asks.state === 'unavailable') {
    emptyMessage = 'Ask next unavailable for this call.';
  } else if (!hasItems && asks.updatedAt === null) {
    emptyMessage = 'Listening. Questions appear after the first exchange.';
  } else if (!hasItems) {
    emptyMessage = 'Nothing to ask right now.';
  }

  return (
    <div className="max-h-80 overflow-y-auto border-b border-gray-200 p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5">
          <span className="text-xs font-medium text-gray-500">Ask next</span>
          {asks.checking && (
            <span className="flex items-center gap-1 text-[11px] text-gray-400">
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-amber-400" />
              Checking
            </span>
          )}
          {asks.checked && <span className="text-[11px] text-gray-400">Checked</span>}
        </div>
        <div className="flex items-center gap-2">
          <span className={failed ? 'text-xs text-amber-600' : 'text-xs text-gray-400'}>
            {failed ? 'refresh failed' : formatUpdatedAgo(asks.updatedAt, now)}
          </span>
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            onClick={refreshAsks}
            disabled={disabled || asks.state === 'unavailable'}
            title="Refresh (Option-Q)"
            aria-label="Refresh Ask next"
          >
            {thinking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
          </Button>
        </div>
      </div>

      {emptyMessage ? (
        <p className="text-xs text-gray-400">{emptyMessage}</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {asks.asks.map((item) => (
            <li key={item.id} className="group flex items-start gap-2">
              <div
                className={cn(
                  'flex-1 rounded transition-colors duration-1000',
                  isChanged(item.id) && 'bg-blue-50'
                )}
              >
                <p className="text-sm text-gray-900">{item.question}</p>
                {item.why && <p className="text-xs text-gray-400">{item.why}</p>}
              </div>
              <button
                type="button"
                onClick={() => dismissAsk(item.id)}
                title="Dismiss"
                aria-label={`Dismiss ${item.question}`}
                disabled={disabled}
                className="mt-0.5 text-gray-300 hover:text-gray-500 disabled:cursor-not-allowed disabled:opacity-50"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </li>
          ))}
        </ul>
      )}

      {asks.forYou.length > 0 && (
        <div className="mt-3 border-t border-gray-100 pt-2">
          <span className="text-xs font-medium text-gray-500">For you</span>
          <ul className="mt-1 flex flex-col gap-2">
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
    </div>
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
    <li className="flex flex-col gap-1">
      <div className="flex items-start gap-2">
        <p className={cn('flex-1 rounded text-sm text-gray-900 transition-colors duration-1000', changed && 'bg-blue-50')}>
          {item.question}
        </p>
        <button
          type="button"
          onClick={onDismiss}
          title="Dismiss"
          aria-label={`Dismiss ${item.question}`}
          disabled={disabled}
          className="mt-0.5 text-gray-300 hover:text-gray-500 disabled:cursor-not-allowed disabled:opacity-50"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
      <Input
        aria-label={`Answer: ${item.question}`}
        disabled={disabled}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Answer, then Enter"
        className="h-7 text-xs"
      />
    </li>
  );
}
