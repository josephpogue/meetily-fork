/**
 * Meeting detect service
 *
 * Tauri commands and events for the per-app meeting prompt, plus the one
 * place that remembers which call app a manual start records.
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { CallApp, CallAppStatus, MeetingDetectedPayload } from '@/types';

class MeetingDetectService {
  listCallApps(): Promise<CallAppStatus[]> {
    return invoke<CallAppStatus[]>('list_call_apps');
  }

  current(): Promise<MeetingDetectedPayload[]> {
    return invoke<MeetingDetectedPayload[]>('meeting_detect_current');
  }

  dismiss(app: CallApp): Promise<void> {
    return invoke('meeting_detect_dismiss', { app });
  }

  onDetected(cb: (p: MeetingDetectedPayload) => void): Promise<UnlistenFn> {
    return listen<MeetingDetectedPayload>('meeting-detected', (e) => cb(e.payload));
  }

  onCleared(cb: (p: { app: CallApp }) => void): Promise<UnlistenFn> {
    return listen<{ app: CallApp }>('meeting-detected-cleared', (e) => cb(e.payload));
  }

  onRecord(cb: (p: MeetingDetectedPayload) => void): Promise<UnlistenFn> {
    return listen<MeetingDetectedPayload>('meeting-detect-record', (e) => cb(e.payload));
  }
}

export const meetingDetectService = new MeetingDetectService();

// The call app a manual start records. Module-level so the picker, the Start
// button's hook and the sidebar start event all read the same value.
let callAudio: CallApp | null = null;
const callAudioListeners = new Set<() => void>();

export const callAudioStore = {
  get: (): CallApp | null => callAudio,
  set(value: CallApp | null) {
    if (value === callAudio) return;
    callAudio = value;
    callAudioListeners.forEach((l) => l());
  },
  subscribe(l: () => void) {
    callAudioListeners.add(l);
    return () => {
      callAudioListeners.delete(l);
    };
  },
};

/** Window event a card's Record raises; same path as the sidebar's start. */
export const START_FROM_SIDEBAR_EVENT = 'start-recording-from-sidebar';

/** sessionStorage key a card's Record sets before sending the user to the home page. */
export const AUTO_START_CALL_APP_KEY = 'autoStartCallApp';
