'use client';

import React, { createContext, useContext, useState, useEffect, useCallback, useMemo, useRef } from 'react';
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

const NO_IDS: ReadonlySet<string> = new Set();
const INITIAL_CARDS: AssistantCard[] = [];

const DEFAULT_VOICE: AssistantVoice = { state: 'off', heard: '' };
const DEFAULT_NOTE: AssistantNote = { state: 'idle', markdown: '', error: null };
const DEFAULT_ASKS: AssistantAsks = {
  state: 'idle',
  asks: [],
  forYou: [],
  answered: [],
  updatedAt: null,
  brief: '',
  checked: false,
  checking: false,
  changed: [],
  todos: [],
  dismissedTodos: [],
};

interface AssistantContextType {
  status: AssistantStatus;
  cards: AssistantCard[];
  voice: AssistantVoice;
  note: AssistantNote;
  asks: AssistantAsks;
  refreshAsks: () => Promise<void>;
  dismissAsk: (id: string) => Promise<void>;
  /** Add a typed to-do. Resolves with null on success, or the command's error message. */
  addTodo: (text: string) => Promise<string | null>;
  dismissTodo: (id: string) => Promise<void>;
  /**
   * Delete items by their live ids. They leave every list at once and stay out
   * for the session, whatever the backend keeps sending. Resolves true when the
   * backend deleted them; on failure the items come back and it resolves false.
   */
  deleteItems: (ids: string[]) => Promise<boolean>;
  /** Joseph's answer to a For-you question; the backend moves it to the answered list. */
  answerAsk: (id: string, answer: string) => Promise<void>;
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
  /** Files the drafted note under the chosen vault project slug, or null for none. */
  saveNote: (project: string | null) => Promise<void>;
  /** The project the note was last filed under: undefined before a save, null for none. */
  savedNoteProject: string | null | undefined;
  /** Why the last Save failed, or null. The note stays ready so Joseph can pick again. */
  saveNoteError: string | null;
  /** The vault's project slugs, or null when they could not be read. */
  listVaultProjects: () => Promise<string[] | null>;
  suggestVaultProject: () => Promise<string | null>;
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
  const [voice, setVoice] = useState<AssistantVoice>(DEFAULT_VOICE);
  const [note, setNote] = useState<AssistantNote>(DEFAULT_NOTE);
  const [savedNoteProject, setSavedNoteProject] = useState<string | null | undefined>(undefined);
  const [saveNoteError, setSaveNoteError] = useState<string | null>(null);
  const [rawAsks, setAsks] = useState<AssistantAsks>(DEFAULT_ASKS);
  const [rawCards, setCards] = useState<AssistantCard[]>(INITIAL_CARDS);
  // Ids Joseph deleted this session. The backend keeps sending some of them (an
  // answered question stays in the lane's snapshots, a drafting card is emitted
  // again when checked), so every list is filtered through this set.
  const [deletedIds, setDeletedIds] = useState<ReadonlySet<string>>(NO_IDS);
  const [brief, setBriefState] = useState<BriefState>({ draft: DEFAULT_ASKS.brief, saved: DEFAULT_ASKS.brief });
  const [backendBrief, setBackendBrief] = useState(DEFAULT_ASKS.brief);

  // Take the backend's latest copy of the brief as the saved baseline; the
  // draft adopts it, or absorbs an appended For-you answer, per reconcileBrief.
  if (rawAsks.brief !== backendBrief) {
    setBackendBrief(rawAsks.brief);
    setBriefState(reconcileBrief(brief, rawAsks.brief));
  }

  const cards = useMemo(
    () => (deletedIds.size === 0 ? rawCards : rawCards.filter(c => !deletedIds.has(c.id))),
    [rawCards, deletedIds]
  );
  const asks = useMemo<AssistantAsks>(() => {
    if (deletedIds.size === 0) return rawAsks;
    const keep = <T extends { id: string }>(list: T[]) => list.filter(item => !deletedIds.has(item.id));
    return {
      ...rawAsks,
      asks: keep(rawAsks.asks),
      forYou: keep(rawAsks.forYou),
      answered: keep(rawAsks.answered),
      // An older payload has no todos field: leave it as it came.
      todos: rawAsks.todos ? keep(rawAsks.todos) : rawAsks.todos,
    };
  }, [rawAsks, deletedIds]);

  // A new recording starts a new session with new ids: the deleted cards leave
  // for good and the deleted set resets.
  const wasSessionOpen = useRef(false);
  const deletedNow = useRef(deletedIds);
  deletedNow.current = deletedIds;
  useEffect(() => {
    if (status.sessionOpen && !wasSessionOpen.current) {
      setCards(prev => prev.filter(c => !deletedNow.current.has(c.id)));
      setDeletedIds(NO_IDS);
    }
    wasSessionOpen.current = status.sessionOpen;
  }, [status.sessionOpen]);

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
          // A new draft is a new note: it shows neither the last note's saved
          // project nor the last save's error.
          if (next.state === 'drafting') {
            setSavedNoteProject(undefined);
            setSaveNoteError(null);
          }
        });
        unsubscribers.push(unlistenNote);

        const unlistenAsks = await assistantService.onAsks((next) => {
          setAsks(next);
        });
        unsubscribers.push(unlistenAsks);

        // Deletes made on the Assistant tab: the same live ids leave this panel.
        const unlistenDeleted = await assistantService.onItemsDeleted(({ ids }) => {
          setDeletedIds(prev => new Set([...prev, ...ids]));
        });
        unsubscribers.push(unlistenDeleted);

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

  const saveNote = useCallback(async (project: string | null) => {
    setSaveNoteError(null);
    try {
      const saved = await assistantService.saveNote(project);
      setSavedNoteProject(saved.project);
    } catch (error) {
      console.error('[AssistantContext] assistant_save_note failed:', error);
      setSaveNoteError(typeof error === 'string' ? error : error instanceof Error ? error.message : 'Unknown error.');
    }
  }, []);

  const listVaultProjects = useCallback(async () => {
    try {
      return await assistantService.listVaultProjects();
    } catch (error) {
      console.error('[AssistantContext] assistant_list_vault_projects failed:', error);
      return null;
    }
  }, []);

  const suggestVaultProject = useCallback(async () => {
    try {
      return await assistantService.suggestVaultProject();
    } catch (error) {
      console.error('[AssistantContext] assistant_suggest_vault_project failed:', error);
      return null;
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

  const addTodo = useCallback(async (text: string): Promise<string | null> => {
    try {
      await assistantService.addTodo(text);
      return null;
    } catch (error) {
      console.error('[AssistantContext] assistant_add_todo failed:', error);
      if (typeof error === 'string') return error;
      return error instanceof Error ? error.message : "Couldn't add that to-do.";
    }
  }, []);

  const dismissTodo = useCallback(async (id: string) => {
    try {
      await assistantService.dismissTodo(id);
    } catch (error) {
      console.error('[AssistantContext] assistant_dismiss_todo failed:', error);
    }
  }, []);

  const deleteItems = useCallback(async (ids: string[]): Promise<boolean> => {
    if (ids.length === 0) return true;
    // Gone at once; a failed command brings them back below.
    setDeletedIds(prev => new Set([...prev, ...ids]));
    try {
      await assistantService.deleteItems(ids);
      return true;
    } catch (error) {
      console.error('[AssistantContext] assistant_delete_items failed:', error);
      setDeletedIds(prev => new Set([...prev].filter(id => !ids.includes(id))));
      return false;
    }
  }, []);

  const answerAsk = useCallback(async (id: string, answer: string) => {
    try {
      await assistantService.answerAsk(id, answer);
    } catch (error) {
      console.error('[AssistantContext] assistant_answer_ask failed:', error);
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
    addTodo,
    dismissTodo,
    deleteItems,
    answerAsk,
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
    savedNoteProject,
    saveNoteError,
    listVaultProjects,
    suggestVaultProject,
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
