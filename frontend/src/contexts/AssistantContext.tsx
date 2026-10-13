'use client';

import React, { createContext, useContext, useState, useEffect, useCallback } from 'react';
import { AssistantCard, AssistantStatus, AssistantVoice, AssistantNote, AssistantAsks } from '@/types';
import { assistantService } from '@/services/assistantService';
import { reconcileBrief, type BriefState } from '@/lib/ask-next';

/**
 * Live assistant state synchronized with backend.
 * Every backend call is wrapped so a missing or failing command never breaks the
 * panel: the assistant module can be absent (backend lane mid-build) or disabled,
 * and the UI still renders its empty states.
 */

const DEFAULT_STATUS: AssistantStatus = {
  enabled: false,
  sessionOpen: false,
  lanesReady: false,
  mode: 'gated',
  listening: false,
  claudeOk: true,
  lastError: null,
  lastErrorSeq: 0,
};

const DEFAULT_VOICE: AssistantVoice = { state: 'off', heard: '' };
const DEFAULT_NOTE: AssistantNote = { state: 'idle', markdown: '', error: null };
const DEFAULT_ASKS: AssistantAsks = { state: 'idle', asks: [], forYou: [], updatedAt: null, brief: '' };

interface AssistantContextType {
  status: AssistantStatus;
  cards: AssistantCard[];
  voice: AssistantVoice;
  note: AssistantNote;
  asks: AssistantAsks;
  refreshAsks: () => Promise<void>;
  dismissAsk: (id: string) => Promise<void>;
  ask: (text: string) => Promise<void>;
  explain: () => Promise<void>;
  catchup: () => Promise<void>;
  setMode: (mode: string) => Promise<void>;
  setListening: (listening: boolean) => Promise<void>;
  setEnabled: (enabled: boolean) => Promise<void>;
  voiceStart: () => Promise<void>;
  voiceFinish: () => Promise<void>;
  voiceCancel: () => Promise<void>;
  draftNote: () => Promise<void>;
  saveNote: () => Promise<void>;
  discardNote: () => Promise<void>;
  setBrief: (text: string) => Promise<void>;
  /** The brief box's text, kept here so it survives the box unmounting. */
  briefDraft: string;
  setBriefDraft: (text: string) => void;
  /** The last brief known to be saved; the draft is unsaved while it differs. */
  briefSaved: string;
  /** Send the draft to the backend as the brief. No-op when nothing is unsaved. */
  saveBrief: () => void;
}

const AssistantContext = createContext<AssistantContextType | null>(null);

export const useAssistant = () => {
  const context = useContext(AssistantContext);
  if (!context) {
    throw new Error('useAssistant must be used within an AssistantProvider');
  }
  return context;
};

export function AssistantProvider({ children }: { children: React.ReactNode }) {
  const [status, setStatus] = useState<AssistantStatus>(DEFAULT_STATUS);
  const [cards, setCards] = useState<AssistantCard[]>([]);
  const [voice, setVoice] = useState<AssistantVoice>(DEFAULT_VOICE);
  const [note, setNote] = useState<AssistantNote>(DEFAULT_NOTE);
  const [asks, setAsks] = useState<AssistantAsks>(DEFAULT_ASKS);
  const [brief, setBriefState] = useState<BriefState>({ draft: DEFAULT_ASKS.brief, saved: DEFAULT_ASKS.brief });
  const [backendBrief, setBackendBrief] = useState(DEFAULT_ASKS.brief);

  // Take the backend's latest copy of the brief as the saved baseline; the
  // draft adopts it, or absorbs an appended For-you answer, per reconcileBrief.
  if (asks.brief !== backendBrief) {
    setBackendBrief(asks.brief);
    setBriefState(reconcileBrief(brief, asks.brief));
  }

  // Initial state fetch. A missing backend command (Rust module not built yet,
  // or the assistant disabled) leaves status at its off-by-default value.
  useEffect(() => {
    const loadInitialState = async () => {
      try {
        const initial = await assistantService.getState();
        setStatus(initial);
      } catch (error) {
        console.warn('[AssistantContext] assistant_get_state unavailable, using defaults:', error);
      }
    };

    loadInitialState();
  }, []);

  // Event listeners. Set up once for the app lifetime, matching OllamaDownloadContext.
  useEffect(() => {
    console.log('[AssistantContext] Setting up event listeners');
    const unsubscribers: (() => void)[] = [];

    const setupListeners = async () => {
      try {
        const unlistenCard = await assistantService.onCard((card) => {
          setCards(prev => {
            const idx = prev.findIndex(c => c.id === card.id);
            if (idx !== -1) {
              const updated = [...prev];
              updated[idx] = card;
              return updated;
            }
            return [card, ...prev];
          });
        });
        unsubscribers.push(unlistenCard);

        const unlistenStatus = await assistantService.onStatus((next) => {
          setStatus(next);
        });
        unsubscribers.push(unlistenStatus);

        const unlistenVoice = await assistantService.onVoice((next) => {
          setVoice(next);
        });
        unsubscribers.push(unlistenVoice);

        const unlistenNote = await assistantService.onNote((next) => {
          setNote(next);
        });
        unsubscribers.push(unlistenNote);

        const unlistenAsks = await assistantService.onAsks((next) => {
          setAsks(next);
        });
        unsubscribers.push(unlistenAsks);

        console.log('[AssistantContext] Event listeners set up successfully');
      } catch (error) {
        console.error('[AssistantContext] Failed to set up event listeners:', error);
      }
    };

    setupListeners();

    return () => {
      console.log('[AssistantContext] Cleaning up event listeners');
      unsubscribers.forEach(unsub => unsub());
    };
  }, []);

  const ask = useCallback(async (text: string) => {
    try {
      await assistantService.ask(text);
    } catch (error) {
      console.error('[AssistantContext] assistant_ask failed:', error);
    }
  }, []);

  const explain = useCallback(async () => {
    try {
      await assistantService.explain();
    } catch (error) {
      console.error('[AssistantContext] assistant_explain failed:', error);
    }
  }, []);

  const catchup = useCallback(async () => {
    try {
      await assistantService.catchup();
    } catch (error) {
      console.error('[AssistantContext] assistant_catchup failed:', error);
    }
  }, []);

  const setMode = useCallback(async (mode: string) => {
    try {
      await assistantService.setMode(mode);
    } catch (error) {
      console.error('[AssistantContext] assistant_set_mode failed:', error);
    }
  }, []);

  const setListening = useCallback(async (listening: boolean) => {
    try {
      await assistantService.setListening(listening);
    } catch (error) {
      console.error('[AssistantContext] assistant_set_listening failed:', error);
    }
  }, []);

  const setEnabled = useCallback(async (enabled: boolean) => {
    try {
      await assistantService.setEnabled(enabled);
      // Optimistic local update so the panel reacts even before the backend
      // status event round-trips (or when the command silently no-ops absent).
      setStatus(prev => ({ ...prev, enabled }));
    } catch (error) {
      console.error('[AssistantContext] assistant_set_enabled failed:', error);
    }
  }, []);

  const voiceStart = useCallback(async () => {
    try {
      await assistantService.voiceStart();
    } catch (error) {
      console.error('[AssistantContext] assistant_voice_start failed:', error);
    }
  }, []);

  const voiceFinish = useCallback(async () => {
    try {
      await assistantService.voiceFinish();
    } catch (error) {
      console.error('[AssistantContext] assistant_voice_finish failed:', error);
    }
  }, []);

  const voiceCancel = useCallback(async () => {
    try {
      await assistantService.voiceCancel();
    } catch (error) {
      console.error('[AssistantContext] assistant_voice_cancel failed:', error);
    }
  }, []);

  const draftNote = useCallback(async () => {
    try {
      await assistantService.draftNote();
    } catch (error) {
      console.error('[AssistantContext] assistant_draft_note failed:', error);
    }
  }, []);

  const saveNote = useCallback(async () => {
    try {
      await assistantService.saveNote();
    } catch (error) {
      console.error('[AssistantContext] assistant_save_note failed:', error);
    }
  }, []);

  const discardNote = useCallback(async () => {
    try {
      await assistantService.discardNote();
    } catch (error) {
      console.error('[AssistantContext] assistant_discard_note failed:', error);
    }
  }, []);

  const setBrief = useCallback(async (text: string) => {
    try {
      await assistantService.setBrief(text);
    } catch (error) {
      console.error('[AssistantContext] assistant_set_brief failed:', error);
    }
  }, []);

  const setBriefDraft = useCallback((text: string) => {
    setBriefState(prev => ({ ...prev, draft: text }));
  }, []);

  const saveBrief = useCallback(() => {
    if (brief.draft === brief.saved) return;
    void setBrief(brief.draft);
    // Optimistic: set_brief outside a session may emit no event to echo it.
    setBriefState(prev => ({ ...prev, saved: brief.draft }));
  }, [brief, setBrief]);

  const refreshAsks = useCallback(async () => {
    try {
      await assistantService.refreshAsks();
    } catch (error) {
      console.error('[AssistantContext] assistant_refresh_asks failed:', error);
    }
  }, []);

  const dismissAsk = useCallback(async (id: string) => {
    try {
      await assistantService.dismissAsk(id);
    } catch (error) {
      console.error('[AssistantContext] assistant_dismiss_ask failed:', error);
    }
  }, []);

  const value: AssistantContextType = {
    status,
    cards,
    voice,
    note,
    asks,
    refreshAsks,
    dismissAsk,
    ask,
    explain,
    catchup,
    setMode,
    setListening,
    setEnabled,
    voiceStart,
    voiceFinish,
    voiceCancel,
    draftNote,
    saveNote,
    discardNote,
    setBrief,
    briefDraft: brief.draft,
    setBriefDraft,
    briefSaved: brief.saved,
    saveBrief,
  };

  return (
    <AssistantContext.Provider value={value}>
      {children}
    </AssistantContext.Provider>
  );
}
