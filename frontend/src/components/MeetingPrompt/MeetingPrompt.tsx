'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { Video } from 'lucide-react';
import { usePathname, useRouter } from 'next/navigation';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import {
  callAudioStore,
  meetingDetectService,
  START_FROM_SIDEBAR_EVENT,
  AUTO_START_CALL_APP_KEY,
} from '@/services/meetingDetectService';
import type { CallApp, MeetingDetectedPayload } from '@/types';

const ORDER: CallApp[] = ['zoom', 'teams'];

/** One card per detected call app while Meetily is not recording. */
export function MeetingPrompt() {
  const { isRecording } = useRecordingState();
  const pathname = usePathname();
  const router = useRouter();
  const [calls, setCalls] = useState<Partial<Record<CallApp, MeetingDetectedPayload>>>({});
  const routeRef = useRef({ pathname, router });
  routeRef.current = { pathname, router };
  const isRecordingRef = useRef(isRecording);
  const wasRecording = useRef(isRecording);

  useEffect(() => {
    isRecordingRef.current = isRecording;
    if (isRecording) {
      setCalls({});
    } else if (wasRecording.current) {
      callAudioStore.set(null); // After Stop the picker is back on All apps.
    }
    wasRecording.current = isRecording;
  }, [isRecording]);

  const record = useCallback((app: CallApp) => {
    if (isRecordingRef.current) return;
    const { pathname: here, router: nav } = routeRef.current;
    if (here !== '/') {
      // The start flow lives on the home page: go there and let its auto-start take over.
      sessionStorage.setItem('autoStartRecording', String(Date.now()));
      sessionStorage.setItem(AUTO_START_CALL_APP_KEY, app);
      nav.push('/');
      return;
    }
    window.dispatchEvent(new CustomEvent(START_FROM_SIDEBAR_EVENT, { detail: { callApp: app } }));
  }, []);

  useEffect(() => {
    let alive = true;
    const unlistens: Array<() => void> = [];
    const keep = (p: Promise<() => void>) =>
      p.then((fn) => (alive ? unlistens.push(fn) : fn())).catch((e) => console.error('[MeetingPrompt] listen failed', e));

    keep(meetingDetectService.onDetected((p) => {
      if (isRecordingRef.current) return;
      setCalls((prev) => ({ ...prev, [p.app]: p }));
    }));
    keep(meetingDetectService.onCleared(({ app }) => {
      setCalls((prev) => {
        if (!prev[app]) return prev;
        const next = { ...prev };
        delete next[app];
        return next;
      });
    }));
    keep(meetingDetectService.onRecord((p) => record(p.app)));

    meetingDetectService.current().then((list) => {
      if (!alive || isRecordingRef.current || !Array.isArray(list)) return;
      setCalls((prev) => {
        const next = { ...prev };
        for (const p of list) if (!next[p.app]) next[p.app] = p;
        return next;
      });
    }).catch((e) => console.error('[MeetingPrompt] meeting_detect_current failed', e));

    return () => {
      alive = false;
      unlistens.forEach((fn) => fn());
    };
  }, [record]);

  const dismiss = useCallback((app: CallApp) => {
    setCalls((prev) => {
      const next = { ...prev };
      delete next[app];
      return next;
    });
    meetingDetectService.dismiss(app).catch((e) => console.error('[MeetingPrompt] dismiss failed', e));
  }, []);

  const cards = ORDER.map((a) => calls[a]).filter((c): c is MeetingDetectedPayload => !!c);
  if (isRecording || cards.length === 0) return null;

  return (
    <div
      className="fixed top-4 z-40 flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2"
      // Sit left of the assistant panel, but never push the cards off a narrow window.
      style={{ right: 'max(1rem, min(calc(1rem + var(--assistant-panel-width, 0px)), calc(100vw - 21rem)))' }}
      role="region"
      aria-label="Detected meetings"
    >
      {cards.map((c) => (
        <section
          key={c.app}
          aria-label={`${c.label} meeting`}
          className="rounded-lg border border-gray-200 bg-white p-4 shadow-lg"
        >
          <div className="flex items-start gap-3">
            <Video size={20} className="mt-0.5 shrink-0 text-blue-600" aria-hidden="true" />
            <div className="min-w-0">
              <p className="text-sm font-semibold text-gray-900">{c.label} meeting started</p>
              {c.title && <p className="truncate text-sm text-gray-600">{c.title}</p>}
            </div>
          </div>
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              onClick={() => record(c.app)}
              aria-label={`Record ${c.label} meeting`}
              className="rounded-md bg-red-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-red-700 focus:outline-none focus-visible:ring-2 focus-visible:ring-red-500 focus-visible:ring-offset-2"
            >
              Record {c.label} meeting
            </button>
            <button
              type="button"
              onClick={() => dismiss(c.app)}
              aria-label={`Dismiss ${c.label} meeting`}
              className="rounded-md border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700 transition-colors hover:bg-gray-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-2"
            >
              Dismiss
            </button>
          </div>
        </section>
      ))}
    </div>
  );
}
