'use client';

import { useMemo, useState } from 'react';
import { CornerDownRight } from 'lucide-react';
import { useAssistant } from '@/contexts/AssistantContext';
import { cn } from '@/lib/utils';
import { AssistantCard } from './AssistantCard';
import { LaneAll, useSelection } from './selection';
import { LANE_TONE } from './KindChip';

type Filter = 'all' | 'answers' | 'mine';

const FILTERS: { id: Filter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'answers', label: 'Answers' },
  { id: 'mine', label: 'Mine' },
];

/**
 * The Say lane: what Joseph could say. The newest answer card is pinned at the
 * top, then earlier answers and his own chat (Ask, Explain, Catch up), newest
 * first, with All / Answers / Mine filters on the earlier list.
 */
export function SayLane() {
  const { cards, status } = useAssistant();
  const [chosenFilter, setFilter] = useState<Filter>('all');
  // Select mode shows every card; the chosen filter comes back on leaving it.
  const { selecting } = useSelection();
  const filter: Filter = selecting ? 'all' : chosenFilter;

  const { pinned, earlier } = useMemo(() => {
    const newestFirst = [...cards].sort((a, b) => b.ts - a.ts);
    const pinnedCard = newestFirst.find(c => c.kind === 'answer');
    const rest = newestFirst.filter(c => c !== pinnedCard);
    return {
      pinned: pinnedCard,
      earlier: rest.filter(c =>
        filter === 'all' ? true : filter === 'answers' ? c.kind === 'answer' : c.kind !== 'answer'
      ),
    };
  }, [cards, filter]);

  const warming = status.sessionOpen && !status.lanesReady;

  return (
    <section aria-label="Say" className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-gray-200 py-2.5 pl-3.5 pr-3">
        <span className={cn('grid h-[26px] w-[26px] place-items-center rounded-lg', LANE_TONE.say.badge)}>
          <CornerDownRight className="h-3.5 w-3.5" aria-hidden="true" />
        </span>
        <h2 className={cn('flex-1 text-[15px] font-bold', LANE_TONE.say.title)}>Say</h2>
        {selecting ? (
          <LaneAll ids={cards.map(c => c.id)} label="Select all in Say" />
        ) : (
          <div role="group" aria-label="Show" className="inline-flex gap-0.5 rounded-full bg-gray-100 p-0.5">
            {FILTERS.map(f => (
              <button
                key={f.id}
                type="button"
                aria-pressed={filter === f.id}
                onClick={() => setFilter(f.id)}
                className={cn(
                  'rounded-full px-2.5 py-0.5 text-xs font-semibold',
                  filter === f.id ? 'bg-white text-gray-900 shadow-sm' : 'text-gray-500 hover:text-gray-700'
                )}
              >
                {f.label}
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto bg-gray-50">
        <div className="flex flex-col gap-2.5 p-3">
          {warming && (
            <div role="status" className="flex flex-col gap-2">
              <div className="h-16 animate-pulse rounded-xl bg-gray-200" />
              <div className="h-16 animate-pulse rounded-xl bg-gray-200" />
              <p className="text-xs text-gray-500">Lanes warming up...</p>
            </div>
          )}

          {cards.length === 0 && !warming && (
            <div className="px-1 py-6 text-center">
              <p className="text-sm text-gray-600">No cards yet.</p>
              <p className="mt-1 text-xs text-gray-500">
                Option-A voice ask, Option-E explain, Option-C catch up, Option-M cycle mode, Option-Q refresh questions
              </p>
            </div>
          )}

          {pinned && <AssistantCard card={pinned} pinned />}

          {pinned && (cards.length > 1 || filter !== 'all') && (
            <div className="-mb-0.5 mt-1.5 flex items-center gap-2 text-xs font-semibold text-gray-500">
              Earlier
              <span className="h-px flex-1 bg-gray-200" aria-hidden="true" />
            </div>
          )}

          {earlier.map(card => (
            <AssistantCard key={card.id} card={card} />
          ))}

          {filter !== 'all' && cards.length > 0 && earlier.length === 0 && (
            <p className="px-1 py-2 text-xs text-gray-500">Nothing here yet.</p>
          )}
        </div>
      </div>
    </section>
  );
}
