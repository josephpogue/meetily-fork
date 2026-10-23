'use client';

import { useCallback, useEffect, useState, useSyncExternalStore } from 'react';
import { callAudioStore, meetingDetectService } from '@/services/meetingDetectService';
import type { CallApp, CallAppStatus } from '@/types';

const REFRESH_MS = 10_000;
const DEFAULT_APPS: CallAppStatus[] = [
  { id: 'zoom', label: 'Zoom', running: true },
  { id: 'teams', label: 'Teams', running: true },
];

/** What a manual start records: all apps, or one call app's sound. */
export function CallAudioPicker({ isRecording }: { isRecording: boolean }) {
  const value = useSyncExternalStore(callAudioStore.subscribe, callAudioStore.get, callAudioStore.get);
  const [apps, setApps] = useState<CallAppStatus[]>(DEFAULT_APPS);

  const refresh = useCallback(() => {
    meetingDetectService.listCallApps()
      .then((list) => { if (Array.isArray(list) && list.length) setApps(list); })
      .catch((e) => console.error('[CallAudioPicker] list_call_apps failed', e));
  }, []);

  useEffect(() => {
    if (isRecording) return;
    refresh();
    const timer = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(timer);
  }, [isRecording, refresh]);

  if (isRecording) {
    const label = value ? `${apps.find((a) => a.id === value)?.label ?? value} only` : 'all apps';
    return <p className="px-2 text-sm text-gray-600">Call audio: {label}</p>;
  }

  return (
    <div className="flex items-center gap-2 px-2">
      <label htmlFor="call-audio-picker" className="text-sm text-gray-600">Call audio</label>
      <select
        id="call-audio-picker"
        value={value ?? ''}
        onChange={(e) => callAudioStore.set((e.target.value || null) as CallApp | null)}
        onFocus={refresh}
        className="rounded-md border border-gray-300 bg-white px-2 py-1 text-sm text-gray-900 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
      >
        <option value="">All apps</option>
        {apps.map((a) => (
          <option key={a.id} value={a.id} disabled={!a.running && value !== a.id}>
            {a.label} only{a.running ? '' : ' (not open)'}
          </option>
        ))}
      </select>
    </div>
  );
}
