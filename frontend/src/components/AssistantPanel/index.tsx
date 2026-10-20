'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import {
  AlertCircle,
  ChevronsLeft,
  ChevronsRight,
  History,
  MessageCircleQuestion,
  Sparkles,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { useAssistant } from '@/contexts/AssistantContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { AskBox } from './AskBox';
import { NoteBar } from './NoteBar';
import { BriefBar } from './BriefBar';
import { BriefFull } from './BriefFull';
import { SayLane } from './SayLane';
import { AskLane } from './AskLane';
import { ScreenRecordingLine } from './ScreenRecordingLine';
import {
  PANEL_KEY_STEP,
  PANEL_MIN_WIDTH,
  clampPanelWidth,
  maxPanelWidth,
  readStoredPanelWidth,
  writeStoredPanelWidth,
  PANEL_DEFAULT_WIDTH,
} from '@/lib/panel-width';

const MODE_ORDER = ['manual', 'gated', 'continuous'] as const;
const MODE_LABELS: Record<(typeof MODE_ORDER)[number], string> = {
  manual: 'Manual',
  gated: 'Gated',
  continuous: 'Continuous',
};

// How long a status.lastError (e.g. "nothing to explain yet") stays visible
// before it clears on its own.
const LAST_ERROR_DISPLAY_MS = 5000;

/**
 * Assistant panel: sibling of TranscriptPanel in the main flex row.
 * Collapsible to a slim rail; renders sanely with the backend absent
 * (AssistantContext defaults status to enabled=false, claudeOk=true).
 */
export function AssistantPanel() {
  const router = useRouter();
  const {
    status,
    cards,
    note,
    voice,
    setEnabled,
    setListening,
    setMode,
    explain,
    catchup,
    voiceCancel,
  } = useAssistant();
  const { isRecording } = useRecordingState();

  const [collapsed, setCollapsed] = useState(false);

  // The full brief takes the Ask lane's column; the Say lane never moves.
  const [briefOpen, setBriefOpen] = useState(false);
  const [briefEditing, setBriefEditing] = useState(false);

  // Width: Joseph's stored choice, clamped to the row that holds the
  // transcript and this panel. A window resize re-clamps the shown width but
  // never rewrites the stored choice.
  const panelRef = useRef<HTMLDivElement>(null);
  const [storedWidth, setStoredWidth] = useState<number | null>(null);
  const [rowWidth, setRowWidth] = useState(10_000);
  const [dragging, setDragging] = useState(false);
  const dragStart = useRef<{ x: number; width: number } | null>(null);

  useEffect(() => {
    setStoredWidth(readStoredPanelWidth());
  }, []);

  // Re-observed when the panel leaves or returns from the collapsed rail.
  useEffect(() => {
    const row = panelRef.current?.parentElement;
    if (!row) return;
    const measure = () => setRowWidth(row.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(row);
    return () => observer.disconnect();
  }, [collapsed]);

  const width = clampPanelWidth(storedWidth, rowWidth);

  // The recording controls float over the page; app/page.tsx keeps them
  // clear of this panel by reading its current width from this variable.
  useEffect(() => {
    const root = document.documentElement;
    root.style.setProperty('--assistant-panel-width', `${collapsed ? 40 : width}px`);
    return () => {
      root.style.removeProperty('--assistant-panel-width');
    };
  }, [collapsed, width]);

  const commitWidth = useCallback((next: number) => {
    const clamped = clampPanelWidth(next, rowWidth);
    setStoredWidth(clamped);
    return clamped;
  }, [rowWidth]);

  const onHandlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    dragStart.current = { x: e.clientX, width };
    setDragging(true);
  };
  const onHandlePointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const start = dragStart.current;
    if (!start) return;
    commitWidth(start.width + (start.x - e.clientX));
  };
  const endDrag = () => {
    if (!dragStart.current) return;
    dragStart.current = null;
    setDragging(false);
    writeStoredPanelWidth(width);
  };
  const onHandleKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    const delta = e.key === 'ArrowLeft' ? PANEL_KEY_STEP : -PANEL_KEY_STEP;
    writeStoredPanelWidth(commitWidth(width + delta));
  };
  const resetWidth = () => {
    setStoredWidth(PANEL_DEFAULT_WIDTH);
    writeStoredPanelWidth(PANEL_DEFAULT_WIDTH);
  };

  // No text selection while the handle is dragged.
  useEffect(() => {
    if (!dragging) return;
    const previous = document.body.style.userSelect;
    document.body.style.userSelect = 'none';
    return () => {
      document.body.style.userSelect = previous;
    };
  }, [dragging]);

  // Tracks whether a recording has actually run this panel's lifetime, so the
  // end-of-meeting note bar can appear after that recording stops even when
  // no cards were ever generated. Local, not backend-derived: status.sessionOpen
  // is not a reliable signal for this yet.
  const [hasRecorded, setHasRecorded] = useState(false);
  useEffect(() => {
    if (isRecording) setHasRecorded(true);
  }, [isRecording]);

  // Quiet, transient status for status.lastError (e.g. Explain finding
  // nothing on the Them channel yet): shows the backend's own message,
  // then clears itself after a few seconds or as soon as a card lands.
  //
  // Keyed on lastErrorSeq, not on the lastError string itself: the backend
  // only bumps the seq when a new error is actually set, so two identical
  // error strings in a row (e.g. pressing Explain twice with no Them audio
  // both times) still each show the message. Keying on the string value
  // would silently drop the second one, since it never "changes".
  const [lastErrorMessage, setLastErrorMessage] = useState<string | null>(null);
  const clearLastErrorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!status.lastError || status.lastErrorSeq === 0) return;

    setLastErrorMessage(status.lastError);

    if (clearLastErrorTimer.current) clearTimeout(clearLastErrorTimer.current);
    clearLastErrorTimer.current = setTimeout(() => {
      setLastErrorMessage(null);
    }, LAST_ERROR_DISPLAY_MS);

    return () => {
      if (clearLastErrorTimer.current) clearTimeout(clearLastErrorTimer.current);
    };
  }, [status.lastErrorSeq]);

  const prevCardCountRef = useRef(cards.length);
  useEffect(() => {
    if (cards.length > prevCardCountRef.current) {
      setLastErrorMessage(null);
      if (clearLastErrorTimer.current) clearTimeout(clearLastErrorTimer.current);
    }
    prevCardCountRef.current = cards.length;
  }, [cards.length]);

  // Escape cancels an in-progress voice capture, regardless of what has
  // focus. Only attached while actually listening, so it's never a global
  // key grab and never swallows Escape (modals, etc.) when voice is off.
  useEffect(() => {
    if (voice.state !== 'listening') return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        voiceCancel();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [voice.state, voiceCancel]);

  if (collapsed) {
    return (
      <div ref={panelRef} className="flex w-10 flex-col items-center gap-3 border-l border-gray-200 bg-white py-3">
        <Button
          variant="ghost"
          size="icon"
          onClick={() => setCollapsed(false)}
          title="Expand assistant panel"
        >
          <ChevronsLeft className="h-4 w-4" />
        </Button>
        <Sparkles className={cnStatus(status.enabled)} />
      </div>
    );
  }

  const actionsDisabled = !status.enabled || !status.claudeOk || !status.sessionOpen;

  const cycleMode = () => {
    const idx = MODE_ORDER.indexOf(status.mode);
    const next = MODE_ORDER[(idx + 1) % MODE_ORDER.length];
    setMode(next);
  };

  const openBrief = (editing: boolean) => {
    setBriefOpen(true);
    setBriefEditing(editing);
  };
  const toggleBrief = () => {
    setBriefOpen(!briefOpen);
    setBriefEditing(false);
  };

  return (
    <div
      ref={panelRef}
      style={{ width }}
      className="relative flex shrink-0 flex-col border-l border-gray-200 bg-white"
    >
      {/* Drag handle on the left edge */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize assistant panel"
        aria-valuenow={width}
        aria-valuemin={PANEL_MIN_WIDTH}
        aria-valuemax={maxPanelWidth(rowWidth)}
        tabIndex={0}
        onPointerDown={onHandlePointerDown}
        onPointerMove={onHandlePointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onKeyDown={onHandleKeyDown}
        onDoubleClick={resetWidth}
        className={`group absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize touch-none outline-none ${
          dragging ? 'is-dragging' : ''
        }`}
      >
        <span
          className={`absolute inset-y-0 left-[3px] w-0.5 transition-colors group-hover:bg-blue-400 group-focus-visible:bg-blue-500 ${
            dragging ? 'bg-blue-500' : 'bg-transparent'
          }`}
        />
      </div>

      {/* Header */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-gray-200 px-3.5 py-2.5">
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setListening(!status.listening)}
            title={status.listening ? 'Pause triggers' : 'Resume triggers'}
            className="flex items-center justify-center"
          >
            <span
              className={cnDot(status.enabled && status.listening)}
            />
          </button>
          <span className="text-sm font-semibold text-gray-900">Assistant</span>
          <button
            type="button"
            onClick={cycleMode}
            title="Cycle trigger mode"
            className="rounded-full border border-gray-200 bg-gray-50 px-2 py-0.5 text-xs font-medium text-gray-600 hover:bg-gray-100"
          >
            {MODE_LABELS[status.mode] ?? status.mode}
          </button>
        </div>
        <div className="flex items-center gap-2">
          <Switch checked={status.enabled} onCheckedChange={setEnabled} aria-label="Assistant on or off" />
          <Button variant="ghost" size="icon" onClick={() => setCollapsed(true)} title="Collapse">
            <ChevronsRight className="h-4 w-4" />
          </Button>
        </div>
      </div>

      <ScreenRecordingLine enabled={status.enabled} isRecording={isRecording} />

      {/* Meeting brief preview, editable before and during the call */}
      <BriefBar
        open={briefOpen}
        onToggleOpen={toggleBrief}
        onEdit={() => openBrief(true)}
      />

      {/* Body: empty states or the two lanes */}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {!status.enabled ? (
          <EmptyState
            icon={<Sparkles className="h-5 w-5 text-gray-400" />}
            message="Assistant is off."
            action={<Button size="sm" onClick={() => setEnabled(true)}>Enable</Button>}
          />
        ) : !status.claudeOk ? (
          <EmptyState
            icon={<AlertCircle className="h-5 w-5 text-amber-500" />}
            message="Claude CLI isn't available."
            action={
              <Button size="sm" variant="outline" onClick={() => router.push('/settings')}>
                Open Settings
              </Button>
            }
          />
        ) : (
          <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1.08fr)_minmax(0,1fr)]">
            <div className="flex min-h-0 min-w-0 flex-col overflow-hidden">
              <SayLane />
            </div>
            <div className="flex min-h-0 min-w-0 flex-col overflow-hidden border-l border-gray-200">
              {briefOpen ? (
                <BriefFull
                  editing={briefEditing}
                  onEdit={() => setBriefEditing(true)}
                  onSaved={() => setBriefEditing(false)}
                />
              ) : (
                <AskLane disabled={actionsDisabled} />
              )}
            </div>
          </div>
        )}
      </div>

      {(status.enabled && hasRecorded && !isRecording) || note.state !== 'idle' ? <NoteBar /> : null}

      {lastErrorMessage && (
        <div className="border-t border-gray-200 px-3 py-2 text-center text-xs text-gray-400">
          {lastErrorMessage}
        </div>
      )}

      <div className="shrink-0 border-t border-gray-200 bg-white">
        <div className="flex gap-2 px-3 pt-3">
          <Button
            variant="outline"
            size="sm"
            className="flex-1"
            onClick={explain}
            disabled={actionsDisabled}
            aria-keyshortcuts="Alt+E"
          >
            <MessageCircleQuestion className="h-4 w-4" />
            Explain
            <kbd className="rounded border border-gray-200 bg-gray-50 px-1.5 font-sans text-[11px] text-gray-500" aria-hidden="true">⌥E</kbd>
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="flex-1"
            onClick={catchup}
            disabled={actionsDisabled}
            aria-keyshortcuts="Alt+C"
          >
            <History className="h-4 w-4" />
            Catch up
            <kbd className="rounded border border-gray-200 bg-gray-50 px-1.5 font-sans text-[11px] text-gray-500" aria-hidden="true">⌥C</kbd>
          </Button>
        </div>
        <AskBox disabled={actionsDisabled} />
      </div>
    </div>
  );
}

function EmptyState({
  icon,
  message,
  hint,
  action,
}: {
  icon: React.ReactNode;
  message: string;
  hint?: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
      {icon}
      <p className="text-sm text-gray-500">{message}</p>
      {hint && <p className="text-xs text-gray-400">{hint}</p>}
      {action}
    </div>
  );
}

function cnDot(active: boolean): string {
  return active
    ? 'h-2.5 w-2.5 rounded-full bg-green-500 animate-pulse'
    : 'h-2.5 w-2.5 rounded-full bg-gray-300';
}

function cnStatus(enabled: boolean): string {
  return enabled ? 'h-4 w-4 text-blue-500' : 'h-4 w-4 text-gray-300';
}
