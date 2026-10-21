import { invoke } from '@tauri-apps/api/core';
import type { MeetingAssistantItem, MeetingAssistantMeta } from '@/types';

/** The assistant items saved for past meetings (Rust: api_*_meeting_assistant). */
export const meetingAssistantService = {
  /** The meeting's saved items in call-time order; empty when none were saved. */
  getItems(meetingId: string): Promise<MeetingAssistantItem[]> {
    return invoke<MeetingAssistantItem[]>('api_get_meeting_assistant', { meetingId });
  },

  /** The meeting's brief and where it was filed; empty brief and nulls when there is no record. */
  getMeta(meetingId: string): Promise<MeetingAssistantMeta> {
    return invoke<MeetingAssistantMeta>('api_get_meeting_assistant_meta', { meetingId });
  },

  /** Deletes the saved items with these ids (and rewrites assistant.json and the vault note); resolves with the rows deleted. */
  deleteSelected(meetingId: string, ids: string[]): Promise<number> {
    return invoke<number>('api_delete_meeting_assistant_items', { meetingId, ids });
  },
};
