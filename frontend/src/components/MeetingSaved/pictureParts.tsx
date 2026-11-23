'use client';

import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, ChevronLeft, ChevronRight, RotateCcw, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import type { MeetingPicture, PictureReason } from './allPictures';

// Parts both picture panels share: the picture loader, a thumbnail, the full-size viewer.

/** Why a picture was removed, as the panels word it. */
export const REASON_LABEL: Record<PictureReason, string> = {
  loading: 'Still loading',
  greyed: 'Greyed out while saving or testing',
  blank: 'Blank or no data',
  popup: 'Popup over the page',
  before_share: 'Before anyone shared',
  repeat: 'Repeat',
  by_you: 'Removed by you',
};

export const hhmm = (t: string) => t.slice(0, 5);

export const plural = (n: number) => `${n} ${n === 1 ? 'picture' : 'pictures'}`;

/** "11:47:32" from "teams-frames/teams-20261118-114732-000.jpg". */
export function timeOfFile(file: string | null): string | null {
  const m = file?.match(/-(\d{2})(\d{2})(\d{2})-\d+\.jpg$/i);
  return m ? `${m[1]}:${m[2]}:${m[3]}` : null;
}

/** "Mon, Nov 30" in local time. */
export function dayOf(ms: number): string {
  return new Date(ms).toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' });
}

// ---- Pictures: one command per picture, a few at a time, each fetched once ----

const pictureCache = new Map<string, Promise<string>>();
const waiting: (() => void)[] = [];
let active = 0;

function loadPicture(meetingId: string, file: string): Promise<string> {
  const key = `${meetingId}/${file}`;
  const hit = pictureCache.get(key);
  if (hit) return hit;
  const p = new Promise<string>((resolve, reject) => {
    const run = () => {
      active += 1;
      invoke<string>('picture_cleanup_picture', { meetingId, file })
        .then(resolve, reject)
        .finally(() => {
          active -= 1;
          waiting.shift()?.();
        });
    };
    if (active < 4) run();
    else waiting.push(run);
  });
  p.catch(() => pictureCache.delete(key));
  pictureCache.set(key, p);
  return p;
}

/** A picture; it is fetched only once it is near the screen, so a meeting with 150+ pictures stays light. */
export function Picture({ meetingId, file, alt, className }: { meetingId: string; file: string; alt: string; className: string }) {
  const [src, setSrc] = useState<string | null>(null);
  const [near, setNear] = useState(false);
  const holder = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    setNear(false);
    const el = holder.current;
    if (!el || typeof IntersectionObserver === 'undefined') { setNear(true); return; }
    const seen = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) { setNear(true); seen.disconnect(); }
    }, { rootMargin: '300px' });
    seen.observe(el);
    return () => seen.disconnect();
  }, [meetingId, file]);

  useEffect(() => {
    let cancelled = false;
    setSrc(null);
    if (near) loadPicture(meetingId, file).then((s) => { if (!cancelled) setSrc(s); }).catch(() => {});
    return () => { cancelled = true; };
  }, [meetingId, file, near]);

  if (!src) return <div ref={holder} className={`${className} bg-gray-200`} role="img" aria-label={alt} />;
  return <img src={src} alt={alt} className={className} />;
}

// ---- The full-size viewer ----

/**
 * Steps through `items` with the arrow keys. A removed picture offers Restore. A kept one offers
 * Remove when `onRemove` is given (the All pictures panel), else shows a disabled "Restored".
 * `byYou` maps a kept picture Joseph removed (its `file`) to where it is now, for the repeat pair.
 */
export function Viewer({
  meetingId, items, index, onIndex, onRestore, onRemove, byYou,
}: {
  meetingId: string;
  items: MeetingPicture[];
  index: number | null;
  onIndex: (i: number | null) => void;
  onRestore: (p: MeetingPicture) => void;
  onRemove?: (p: MeetingPicture) => void;
  byYou?: Map<string, string>;
}) {
  const current = index !== null ? items[index] : undefined;

  useEffect(() => {
    if (index === null || items.length === 0) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'ArrowRight') onIndex((index + 1) % items.length);
      else if (e.key === 'ArrowLeft') onIndex((index - 1 + items.length) % items.length);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [index, items.length, onIndex]);

  // The list can shrink under the viewer (the picture it showed was removed): close it past the end.
  useEffect(() => {
    if (index !== null && index >= items.length) onIndex(null);
  }, [index, items.length, onIndex]);

  const keptTime = timeOfFile(current?.repeatOf ?? null);
  const pairedByYou = current?.repeatOf ? byYou?.get(current.repeatOf) : undefined;
  const title = !current
    ? 'Picture'
    : current.reason === 'repeat'
      ? `Repeat of the ${keptTime ?? ''} picture`
      : current.removed
        ? current.caption ?? (current.reason ? REASON_LABEL[current.reason] : 'Removed picture')
        : onRemove ? 'Kept picture' : 'Restored picture';

  const description = !current
    ? ''
    : current.removed
      ? `Removed at ${current.time}.${current.deleteAfterMs ? ` Deleted for good on ${dayOf(current.deleteAfterMs)} unless you restore it.` : ''}`
      : onRemove ? `Kept. Taken at ${current.time}.` : 'Restored. It is back in the recording folder.';

  return (
    <Dialog open={current !== undefined} onOpenChange={(o) => { if (!o) onIndex(null); }}>
      <DialogContent className="max-h-[92vh] w-[min(1400px,95vw)] max-w-none overflow-y-auto">
        <div className="flex items-start justify-between gap-3 pr-8">
          <div className="min-w-0">
            <DialogTitle className="text-base font-semibold text-gray-900">{title}</DialogTitle>
            <DialogDescription className="text-[13px] text-gray-600">{description}</DialogDescription>
          </div>
          <div className="flex flex-shrink-0 items-center gap-2">
            <Button variant="outline" size="icon" aria-label="Previous picture" onClick={() => index !== null && onIndex((index - 1 + items.length) % items.length)}>
              <ChevronLeft />
            </Button>
            <Button variant="outline" size="icon" aria-label="Next picture" onClick={() => index !== null && onIndex((index + 1) % items.length)}>
              <ChevronRight />
            </Button>
            {current && !current.removed && onRemove ? (
              <Button variant="outline" onClick={() => onRemove(current)}>
                <Trash2 /> Remove
              </Button>
            ) : (
              <Button variant="blue" disabled={!current || !current.removed} onClick={() => current && onRestore(current)}>
                <RotateCcw /> {current && !current.removed ? 'Restored' : 'Restore'}
              </Button>
            )}
          </div>
        </div>
        {current && (
          <div className={`grid gap-4 ${current.repeatOf ? 'grid-cols-1 md:grid-cols-2' : 'grid-cols-1'}`}>
            <figure>
              <figcaption className="mb-1 flex items-center gap-2 text-[13px] text-gray-700">
                <span className="inline-flex items-center gap-1 rounded-full bg-gray-100 px-2 py-0.5 font-medium">
                  {current.removed ? <><Trash2 className="h-3 w-3" /> Removed</> : <><Check className="h-3 w-3" /> {onRemove ? 'Kept' : 'Restored'}</>}
                </span>
                {current.time}
              </figcaption>
              <Picture meetingId={meetingId} file={current.shownFile} alt={`Picture at ${current.time}`} className="max-h-[68vh] w-full rounded-md border border-gray-200 object-contain" />
            </figure>
            {current.repeatOf && (
              <figure>
                <figcaption className="mb-1 flex items-center gap-2 text-[13px] text-gray-700">
                  {pairedByYou ? (
                    <span className="inline-flex items-center gap-1 rounded-full bg-gray-100 px-2 py-0.5 font-medium">
                      <Trash2 className="h-3 w-3" /> Removed by you
                    </span>
                  ) : (
                    <span className="inline-flex items-center gap-1 rounded-full bg-green-50 px-2 py-0.5 font-medium text-green-700">
                      <Check className="h-3 w-3" /> Kept
                    </span>
                  )}
                  {keptTime}
                </figcaption>
                <Picture meetingId={meetingId} file={pairedByYou ?? current.repeatOf} alt={`Kept picture at ${keptTime ?? ''}`} className="max-h-[68vh] w-full rounded-md border border-gray-200 object-contain" />
              </figure>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
