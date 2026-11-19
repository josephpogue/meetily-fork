'use client';

import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PictureCleanup } from './PictureCleanup';

// Shapes of api_get_meetings_saved / api_get_meeting_saved (src-tauri/src/meeting_saved.rs).
// The backend writes all the wording; these components only place it.
export interface MeetingSaved {
  brief: string | null;
  line: string;
  durationSeconds: number;
  pictures: number;
  bytes: number;
  /** A picture cleanup record exists for this meeting. */
  cleanup?: boolean;
}

export interface SavedOverview {
  meetingCount: number;
  totals: string;
  meetings: Record<string, MeetingSaved>;
}

/** The meeting's one-line Brief, muted and cut with an ellipsis; nothing when there is none. */
export function BriefLine({ brief, color = 'text-gray-500' }: { brief?: string | null; color?: string }) {
  if (!brief) return null;
  return (
    <p className={`truncate text-xs ${color}`} title={brief}>
      {brief}
    </p>
  );
}

/** "10 meetings, 7.4 hours, 1,204 pictures, 2.1 GB" above the meeting list. */
export function SavedTotals({ totals, meetingCount }: { totals?: string; meetingCount: number }) {
  if (!totals || meetingCount === 0) return null;
  return <p className="px-6 pb-1 text-xs text-gray-500">{totals}</p>;
}

/**
 * Title, Brief and the "What was saved" line at the top of a meeting page, above the page body.
 * While the removed-pictures panel is open it takes the body's place, so both groups and their
 * restore links have the room; the body stays mounted underneath and comes back on Hide.
 * `reloadKey` changes when the meeting list reloads (the provider's `saved`), so a page
 * opened right after Stop picks up the finished numbers.
 */
export function MeetingSavedHeader({
  meetingId, title, reloadKey, children,
}: { meetingId: string; title: string; reloadKey?: unknown; children?: ReactNode }) {
  const [loaded, setLoaded] = useState<{ id: string; saved: MeetingSaved | null } | null>(null);
  // Bumped when picture cleanup finishes or Joseph restores pictures: the saved numbers change.
  const [version, setVersion] = useState(0);
  const onCleanupChanged = useCallback(() => setVersion((n) => n + 1), []);
  const [panelHost, setPanelHost] = useState<HTMLDivElement | null>(null);
  const [panelOpen, setPanelOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<MeetingSaved | null>('api_get_meeting_saved', { meetingId })
      .then((s) => { if (!cancelled) setLoaded({ id: meetingId, saved: s ?? null }); })
      .catch((error) => console.error('Failed to load what was saved:', error));
    return () => { cancelled = true; };
  }, [meetingId, reloadKey, version]);

  // Another meeting's numbers are never shown; the same meeting keeps its line while reloading.
  const saved = loaded?.id === meetingId ? loaded.saved : null;

  return (
    <>
      <div className="flex-shrink-0 min-w-0 border-b border-gray-200 bg-white px-4 py-2">
        <h1 className="truncate text-sm font-semibold text-gray-900" title={title}>{title}</h1>
        <BriefLine brief={saved?.brief} />
        {saved && <p className="text-xs text-gray-600">What was saved: {saved.line}</p>}
        <PictureCleanup
          meetingId={meetingId}
          hasCleanup={saved?.cleanup}
          version={version}
          onChanged={onCleanupChanged}
          panelHost={panelHost}
          onOpenChange={setPanelOpen}
        />
      </div>
      <div ref={setPanelHost} className={panelOpen ? 'min-h-0 min-w-0 flex-1 overflow-y-auto bg-white' : 'hidden'} />
      <div className={panelOpen ? 'hidden' : 'flex min-h-0 min-w-0 flex-1 flex-col'}>{children}</div>
    </>
  );
}
