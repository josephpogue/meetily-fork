'use client';

import { useCallback, useEffect, useRef } from 'react';
import { usePathname, useRouter } from 'next/navigation';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import {
  callAudioStore,
  meetingDetectService,
  START_FROM_SIDEBAR_EVENT,
  AUTO_START_CALL_APP_KEY,
} from '@/services/meetingDetectService';
import type { CallApp } from '@/types';

/**
 * Draws nothing: the meeting cards live in the floating window (FloatingMeetingPrompt). This keeps the
 * main window's half of a Record: on `meeting-detect-record` it starts the recording of that call app.
 */
export function MeetingPrompt({ suppressed = false }: { suppressed?: boolean }) {
  const { isRecording } = useRecordingState();
  const pathname = usePathname();
  const router = useRouter();
  const routeRef = useRef({ pathname, router });
  routeRef.current = { pathname, router };
  // While onboarding shows there is no start listener, so Record starts nothing.
  const suppressedRef = useRef(suppressed);
  suppressedRef.current = suppressed;
  const isRecordingRef = useRef(isRecording);
  const wasRecording = useRef(isRecording);

  useEffect(() => {
    isRecordingRef.current = isRecording;
    if (!isRecording && wasRecording.current) callAudioStore.set(null); // After Stop the picker is back on All apps.
    wasRecording.current = isRecording;
  }, [isRecording]);

  const record = useCallback((app: CallApp) => {
    if (isRecordingRef.current || suppressedRef.current) return;
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
    let unlisten: (() => void) | null = null;
    meetingDetectService
      .onRecord((p) => record(p.app))
      .then((fn) => (alive ? (unlisten = fn) : fn()))
      .catch((e) => console.error('[MeetingPrompt] listen failed', e));
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [record]);

  return null;
}
