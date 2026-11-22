'use client';

import { useCallback, useEffect, useState } from 'react';
import { Video } from 'lucide-react';
import { meetingDetectService } from '@/services/meetingDetectService';
import type { PanelCard } from '@/types';

type Action = 'record' | 'dismiss' | 'keep' | 'stop';

const PRIMARY =
  'rounded-md bg-red-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-red-700 focus:outline-none focus-visible:ring-2 focus-visible:ring-red-500 focus-visible:ring-offset-2';
const SECONDARY =
  'rounded-md border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700 transition-colors hover:bg-gray-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-2';

/**
 * The floating meeting prompt window's content: one card per current prompt, from
 * meetingDetectService.cards() on mount and each `meeting-prompt-cards` payload after.
 * See docs/superpowers/plans/2026-11-21-floating-meeting-prompt-brief.md.
 */
export function FloatingMeetingPrompt(): JSX.Element | null {
  const [cards, setCards] = useState<PanelCard[]>([]);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | null = null;
    meetingDetectService
      .onCards((next) => alive && setCards(next))
      .then((fn) => (alive ? (unlisten = fn) : fn()))
      .catch((e) => console.error('[FloatingMeetingPrompt] listen failed', e));
    meetingDetectService
      .cards()
      .then((list) => alive && Array.isArray(list) && setCards((prev) => (prev.length ? prev : list)))
      .catch((e) => console.error('[FloatingMeetingPrompt] meeting_prompt_cards failed', e));
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  // A press removes nothing: the list is always Rust's latest. Dismiss, Keep and Stop change Rust's state
  // and it sends the new list at once; Record only asks the main window to start, so the card stays
  // (and Record stays pressable) until Rust's state says otherwise. A failed command is only logged.
  const press = useCallback(async (card: PanelCard, action: Action) => {
    try {
      await meetingDetectService[action](card.app);
    } catch (e) {
      console.error(`[FloatingMeetingPrompt] ${action} failed`, e);
    }
  }, []);

  if (cards.length === 0) return null;

  return (
    <div className="flex w-full flex-col gap-2 p-1" role="region" aria-label="Detected meetings">
      {cards.map((c) => (
        <section
          key={`${c.app}-${c.kind}`}
          aria-label={`${c.label} meeting ${c.kind}`}
          className="rounded-lg border border-gray-200 bg-white p-4 shadow-lg"
        >
          <div className="flex items-start gap-3">
            <Video size={20} className="mt-0.5 shrink-0 text-blue-600" aria-hidden="true" />
            <div className="min-w-0">
              <p className="text-sm font-semibold text-gray-900">{c.label} meeting {c.kind}</p>
              {c.title && <p className="truncate text-sm text-gray-600">{c.title}</p>}
            </div>
          </div>
          <div className="mt-3 flex gap-2">
            {c.kind === 'started' ? (
              <>
                <button type="button" className={PRIMARY} onClick={() => press(c, 'record')}>Record</button>
                <button type="button" className={SECONDARY} onClick={() => press(c, 'dismiss')}>Dismiss</button>
              </>
            ) : (
              <>
                <button type="button" className={PRIMARY} onClick={() => press(c, 'stop')}>Stop recording</button>
                <button type="button" className={SECONDARY} onClick={() => press(c, 'keep')}>Keep recording</button>
              </>
            )}
          </div>
        </section>
      ))}
    </div>
  );
}
