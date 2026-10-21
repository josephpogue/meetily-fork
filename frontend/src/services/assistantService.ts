/**
 * Assistant Service
 *
 * Handles all live-assistant Tauri backend calls and events.
 * Pure 1-to-1 wrapper - no error handling changes, exact same behavior as direct invoke/listen calls.
 * Command and event names are copied verbatim from the pinned contract in
 * docs/superpowers/specs/2026-09-01-live-assistant-design.md ("Commands and events").
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import {
  AssistantCard,
  AssistantStatus,
  AssistantVoice,
  AssistantNote,
  AssistantAsks,
  AssistantSettings,
  AssistantClaudeProbe,
} from '@/types';

/** What `assistant_save_note` returns: where the note went and under which project. */
export interface SavedNote {
  notePath: string;
  project: string | null;
}

/**
 * Assistant Service
 * Singleton service for managing live-assistant commands and event subscriptions
 */
export class AssistantService {
  // Commands

  async getState(): Promise<AssistantStatus> {
    return invoke<AssistantStatus>('assistant_get_state');
  }

  async setEnabled(enabled: boolean): Promise<void> {
    return invoke('assistant_set_enabled', { enabled });
  }

  async ask(text: string): Promise<void> {
    return invoke('assistant_ask', { text });
  }

  async explain(): Promise<void> {
    return invoke('assistant_explain');
  }

  async catchup(): Promise<void> {
    return invoke('assistant_catchup');
  }

  async setMode(mode: string): Promise<void> {
    return invoke('assistant_set_mode', { mode });
  }

  async setListening(listening: boolean): Promise<void> {
    return invoke('assistant_set_listening', { listening });
  }

  async voiceStart(): Promise<void> {
    return invoke('assistant_voice_start');
  }

  async voiceFinish(): Promise<void> {
    return invoke('assistant_voice_finish');
  }

  async voiceCancel(): Promise<void> {
    return invoke('assistant_voice_cancel');
  }

  async draftNote(): Promise<void> {
    return invoke('assistant_draft_note');
  }

  /** Files the drafted note under `project` (a vault project slug) or, with null, under none. */
  async saveNote(project: string | null): Promise<SavedNote> {
    return invoke('assistant_save_note', { project });
  }

  /** Slugs of the vault's project pages, sorted. */
  async listVaultProjects(): Promise<string[]> {
    return invoke('assistant_list_vault_projects');
  }

  /** The project the finished meeting most likely belongs to, or null. */
  async suggestVaultProject(): Promise<string | null> {
    return invoke('assistant_suggest_vault_project');
  }

  async discardNote(): Promise<void> {
    return invoke('assistant_discard_note');
  }

  async setBrief(text: string): Promise<void> {
    return invoke('assistant_set_brief', { text });
  }

  async refreshAsks(): Promise<void> {
    return invoke('assistant_refresh_asks');
  }

  async dismissAsk(id: string): Promise<void> {
    return invoke('assistant_dismiss_ask', { id });
  }

  /** Add a to-do Joseph typed (the text after todo:). Rejects with a message when it cannot. */
  async addTodo(text: string): Promise<void> {
    return invoke('assistant_add_todo', { text });
  }

  /** Dismiss a to-do (open or done) from the live list. */
  async dismissTodo(id: string): Promise<void> {
    return invoke('assistant_dismiss_todo', { id });
  }

  /**
   * Delete items by their live ids (cards, Ask, For-you and answered questions,
   * to-dos), during the call and after Stop. Resolves with how many were deleted.
   */
  async deleteItems(ids: string[]): Promise<number> {
    return invoke<number>('assistant_delete_items', { ids });
  }

  /** Joseph answered a For-you question: it moves to the answered list with his answer. */
  async answerAsk(id: string, answer: string): Promise<void> {
    return invoke('assistant_answer_ask', { id, answer });
  }

  async getSettings(): Promise<AssistantSettings> {
    return invoke<AssistantSettings>('assistant_get_settings');
  }

  async saveSettings(settings: AssistantSettings): Promise<void> {
    return invoke('assistant_save_settings', { settings });
  }

  async testClaude(): Promise<AssistantClaudeProbe> {
    return invoke<AssistantClaudeProbe>('assistant_test_claude');
  }

  async screenRecordingAllowed(): Promise<boolean> {
    return invoke<boolean>('assistant_screen_recording_allowed');
  }

  async openScreenRecordingSettings(): Promise<void> {
    return invoke('assistant_open_screen_recording_settings');
  }

  // Event Listeners

  /**
   * Listen for card upserts (drafting, checked, corrected phases)
   */
  async onCard(callback: (card: AssistantCard) => void): Promise<UnlistenFn> {
    return listen<AssistantCard>('assistant-card', (event) => {
      callback(event.payload);
    });
  }

  /**
   * Listen for assistant status changes (enabled, session, lanes, mode, claude health)
   */
  async onStatus(callback: (status: AssistantStatus) => void): Promise<UnlistenFn> {
    return listen<AssistantStatus>('assistant-status', (event) => {
      callback(event.payload);
    });
  }

  /**
   * Listen for voice ask capture state changes
   */
  async onVoice(callback: (voice: AssistantVoice) => void): Promise<UnlistenFn> {
    return listen<AssistantVoice>('assistant-voice', (event) => {
      callback(event.payload);
    });
  }

  /**
   * Listen for note drafting/save state changes
   */
  async onNote(callback: (note: AssistantNote) => void): Promise<UnlistenFn> {
    return listen<AssistantNote>('assistant-note', (event) => {
      callback(event.payload);
    });
  }
  /**
   * Listen for Ask next list changes (thinking, new list, failure, brief sync)
   */
  async onAsks(callback: (asks: AssistantAsks) => void): Promise<UnlistenFn> {
    return listen<AssistantAsks>('assistant-asks', (event) => {
      callback(event.payload);
    });
  }

  /**
   * Listen for deletes made on the Assistant tab: the live ids that were removed,
   * so the live panel drops the same items.
   */
  async onItemsDeleted(callback: (payload: { ids: string[] }) => void): Promise<UnlistenFn> {
    return listen<{ ids: string[] }>('assistant-items-deleted', (event) => {
      callback(event.payload);
    });
  }
}

// Export singleton instance
export const assistantService = new AssistantService();
