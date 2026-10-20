'use client';

import { useEffect, useState } from 'react';
import { Check } from 'lucide-react';
import { AssistantCard as AssistantCardData } from '@/types';
import { cn } from '@/lib/utils';
import { KindChip, type ChipKind } from './KindChip';

interface AssistantCardProps {
  card: AssistantCardData;
  /** The newest answer, shown with the blue "now" look at the top of the Say lane. */
  pinned?: boolean;
}

function formatTime(ts: number): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return '';
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

/** A pulsing amber dot and the word "Drafting". Amber is only used for this and the status line. */
function Drafting() {
  return (
    <span className="inline-flex items-center gap-1 whitespace-nowrap text-xs text-gray-500">
      <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-amber-500" aria-hidden="true" />
      Drafting
    </span>
  );
}

function Phase({ phase }: { phase: AssistantCardData['phase'] }) {
  if (phase === 'drafting') return <Drafting />;
  if (phase === 'corrected') {
    return <span className="whitespace-nowrap text-xs font-semibold text-blue-700">Corrected</span>;
  }
  return (
    <span className="inline-flex items-center gap-1 whitespace-nowrap text-xs text-gray-500">
      <Check className="h-3 w-3" aria-hidden="true" />
      Checked
    </span>
  );
}

/** Briefly highlights the bullets a correction changed (2.5 s), as the old card did. */
function useChangedHighlight(card: AssistantCardData): boolean {
  const isCorrected = card.phase === 'corrected';
  const [highlight, setHighlight] = useState(isCorrected && card.changedLines.length > 0);
  useEffect(() => {
    if (isCorrected && card.changedLines.length > 0) {
      setHighlight(true);
      const timer = setTimeout(() => setHighlight(false), 2500);
      return () => clearTimeout(timer);
    }
    setHighlight(false);
  }, [card.id, card.phase, card.changedLines, isCorrected]);
  return highlight;
}

function Bullets({
  card,
  highlight,
  pinned,
}: {
  card: AssistantCardData;
  highlight: boolean;
  pinned: boolean;
}) {
  if (card.bullets.length === 0) return null;
  return (
    <ul className="mt-1.5 grid list-none gap-1 p-0">
      {card.bullets.map((bullet, i) => (
        <li
          key={i}
          className={cn(
            'relative rounded pl-3.5 leading-snug text-gray-700 transition-colors duration-1000',
            'before:absolute before:left-0.5 before:top-[0.62em] before:h-0.5 before:w-1.5 before:rounded-sm before:content-[""]',
            pinned ? 'text-[14.5px] text-gray-800 before:bg-blue-500' : 'text-sm before:bg-gray-300',
            highlight && card.changedLines.includes(bullet) && 'bg-blue-100'
          )}
        >
          {bullet}
        </li>
      ))}
    </ul>
  );
}

function Source({ card }: { card: AssistantCardData }) {
  if (!card.source) return null;
  return <p className="mt-2 break-words font-mono text-[11px] text-gray-500">{card.source}</p>;
}

const REPLY_CHIP: Record<'ask' | 'explain' | 'catchup', ChipKind> = {
  ask: 'youAsked',
  explain: 'explain',
  catchup: 'catchup',
};

const REPLY_LABEL: Record<'ask' | 'explain' | 'catchup', string> = {
  ask: 'at',
  explain: 'Option-E at',
  catchup: 'Option-C at',
};

/** A suggested answer to a question Joseph was asked. */
function AnswerCard({ card, pinned }: { card: AssistantCardData; pinned: boolean }) {
  const highlight = useChangedHighlight(card);
  return (
    <article
      className={cn(
        'rounded-xl border px-3.5 py-3',
        pinned ? 'border-blue-200 bg-blue-50 shadow-md shadow-blue-900/5' : 'border-gray-200 bg-white'
      )}
    >
      <div className="mb-1.5 flex items-center gap-2">
        <KindChip kind="answer" />
        <span className="min-w-0 flex-1 text-xs tabular-nums text-gray-500">Asked at {formatTime(card.ts)}</span>
        <Phase phase={card.phase} />
      </div>
      {card.question && (
        <p className="mb-1.5 break-words text-[13.5px] italic text-gray-600">&ldquo;{card.question}&rdquo;</p>
      )}
      {card.lead && (
        <p className={cn('break-words font-semibold leading-snug text-gray-900', pinned ? 'text-[16.5px]' : 'text-[15px]')}>
          {card.lead}
        </p>
      )}
      <Bullets card={card} highlight={highlight} pinned={pinned} />
      <Source card={card} />
    </article>
  );
}

/** Joseph's own chat, Explain or Catch up result. */
function ReplyCard({ card }: { card: AssistantCardData }) {
  const highlight = useChangedHighlight(card);
  const kind = card.kind === 'ask' || card.kind === 'catchup' ? card.kind : 'explain';
  return (
    <article className="rounded-xl border border-gray-200 bg-white px-3.5 py-3">
      <div className="mb-1.5 flex items-center gap-2">
        <KindChip kind={REPLY_CHIP[kind]} />
        <span className="min-w-0 flex-1 text-xs tabular-nums text-gray-500">
          {REPLY_LABEL[kind]} {formatTime(card.ts)}
        </span>
        {card.phase === 'drafting' && <Drafting />}
      </div>
      {kind === 'ask' && card.question && (
        <p className="mb-2 inline-block max-w-full break-words rounded-lg rounded-bl-sm bg-slate-100 px-2.5 py-1 text-[13.5px] text-slate-700">
          {card.question}
        </p>
      )}
      {card.lead && <p className="break-words text-[14.5px] font-semibold leading-snug text-gray-900">{card.lead}</p>}
      <Bullets card={card} highlight={highlight} pinned={false} />
      <Source card={card} />
    </article>
  );
}

/** One card of the Say lane: an answer (blue) or one of Joseph's own kinds (slate). */
export function AssistantCard({ card, pinned = false }: AssistantCardProps) {
  return card.kind === 'answer' ? <AnswerCard card={card} pinned={pinned} /> : <ReplyCard card={card} />;
}
