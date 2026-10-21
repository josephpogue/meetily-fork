import { Check, CircleHelp, ClipboardList, CornerDownRight, History, MessageCircleQuestion, MessageSquare, type LucideIcon } from 'lucide-react';
import { cn } from '@/lib/utils';

/**
 * One label, icon and colour per kind of content in the assistant panel, so a
 * glance tells what an item is by colour and by word: blue for what to say,
 * violet for what to ask, green for what got answered, slate for Joseph's own
 * chat, Catch up and Explain, rose for to-dos. Amber is kept for the status line under the header.
 */
export type ChipKind = 'answer' | 'toAsk' | 'answered' | 'youAsked' | 'catchup' | 'explain' | 'todo';

const CHIPS: Record<ChipKind, { label: string; icon: LucideIcon; tone: string }> = {
  answer: { label: 'Answer', icon: CornerDownRight, tone: 'bg-blue-50 text-blue-700 ring-blue-200' },
  toAsk: { label: 'To ask', icon: CircleHelp, tone: 'bg-violet-50 text-violet-700 ring-violet-200' },
  answered: { label: 'Answered', icon: Check, tone: 'bg-emerald-50 text-emerald-700 ring-emerald-200' },
  youAsked: { label: 'You asked', icon: MessageSquare, tone: 'bg-slate-100 text-slate-700 ring-slate-200' },
  catchup: { label: 'Catch up', icon: History, tone: 'bg-slate-100 text-slate-700 ring-slate-200' },
  todo: { label: 'To do', icon: ClipboardList, tone: 'bg-rose-50 text-rose-700 ring-rose-200' },
  explain: { label: 'Explain', icon: MessageCircleQuestion, tone: 'bg-slate-100 text-slate-700 ring-slate-200' },
};

/** Lane accents, for headers and badges that carry a lane's hue. */
export const LANE_TONE = {
  say: { badge: 'bg-blue-600 text-white', title: 'text-blue-700', tint: 'bg-blue-50', line: 'border-blue-200' },
  ask: { badge: 'bg-violet-600 text-white', title: 'text-violet-700', tint: 'bg-violet-50', line: 'border-violet-200' },
  answered: { badge: 'bg-emerald-600 text-white', title: 'text-emerald-700', tint: 'bg-emerald-50', line: 'border-emerald-200' },
} as const;

export function KindChip({ kind, className }: { kind: ChipKind; className?: string }) {
  const { label, icon: Icon, tone } = CHIPS[kind];
  return (
    <span
      className={cn(
        'inline-flex h-5 shrink-0 items-center gap-1 whitespace-nowrap rounded-full pl-1.5 pr-2 text-xs font-semibold leading-none ring-1 ring-inset',
        tone,
        className
      )}
    >
      <Icon className="h-3 w-3" aria-hidden="true" />
      {label}
    </span>
  );
}
