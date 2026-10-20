import { invoke } from '@tauri-apps/api/core';
import type { MeetingAssistantItem } from '@/types';

/** The assistant items saved for past meetings (Rust: api_*_meeting_assistant). */
export const meetingAssistantService = {
  /** The meeting's saved items in call-time order; empty when none were saved. */
  getItems(meetingId: string): Promise<MeetingAssistantItem[]> {
    return invoke<MeetingAssistantItem[]>('api_get_meeting_assistant', { meetingId });
  },

  /** Deletes the meeting's saved items and its assistant.json; resolves with the rows deleted. */
  deleteItems(meetingId: string): Promise<number> {
    return invoke<number>('api_delete_meeting_assistant', { meetingId });
  },
};
