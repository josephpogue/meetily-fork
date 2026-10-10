'use client';

import { KeyboardEvent, useEffect, useState } from 'react';
import { Loader2, RefreshCw, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useAssistant } from '@/contexts/AssistantContext';
import { AssistantAskItem } from '@/types';
import { appendBriefAnswer, formatUpdatedAgo } from '@/lib/ask-next';

// How often the "updated 12s ago" stamp re-renders.
const STAMP_TICK_MS = 5000;

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
        <span className="text-xs font-medium text-gray-500">Ask next</span>
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
              <div className="flex-1">
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
  disabled,
  onAnswer,
  onDismiss,
}: {
  item: AssistantAskItem;
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
        <p className="flex-1 text-sm text-gray-900">{item.question}</p>
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
