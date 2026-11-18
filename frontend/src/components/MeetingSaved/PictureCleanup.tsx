'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Check, ChevronDown, ChevronLeft, ChevronRight, ChevronUp, RotateCcw, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';

// Shapes of the picture_cleanup_* commands (src-tauri/src/assistant/cleanup_run.rs).
// The backend writes the wording of the line, the panel title and the dates; this file places it.
type Reason = 'loading' | 'greyed' | 'blank' | 'popup' | 'before_share' | 'repeat';

export interface RemovedPicture {
  file: string;
  removedFile: string;
  time: string;
  reason: Reason;
  caption: string | null;
  repeatOf: string | null;
  bytes: number;
  restored: boolean;
}

export interface CleanupView {
  summary: { line: string | null; failed: boolean; removed: number; repeats: number; notUseful: number; bytes: number; deleteAfterMs: number };
  removed: RemovedPicture[];
  canRetry: boolean;
  running: boolean;
  title: string | null;
  note: string | null;
  bringBack: string | null;
  until: string | null;
}

const CLEANUP_EVENT = 'assistant-cleanup';

const NOT_USEFUL: { reason: Reason; label: string }[] = [
  { reason: 'loading', label: 'Still loading' },
  { reason: 'greyed', label: 'Greyed out while saving or testing' },
  { reason: 'blank', label: 'Blank or no data' },
  { reason: 'popup', label: 'Popup over the page' },
  { reason: 'before_share', label: 'Before anyone shared' },
];

export function isCleanupView(v: unknown): v is CleanupView {
  const o = v as CleanupView | null;
  return !!o && typeof o === 'object' && !!o.summary && typeof o.summary === 'object' && Array.isArray(o.removed);
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

function Picture({ meetingId, file, alt, className }: { meetingId: string; file: string; alt: string; className: string }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    setSrc(null);
    loadPicture(meetingId, file).then((s) => { if (!cancelled) setSrc(s); }).catch(() => {});
    return () => { cancelled = true; };
  }, [meetingId, file]);
  if (!src) return <div className={`${className} bg-gray-200`} role="img" aria-label={alt} />;
  return <img src={src} alt={alt} className={className} />;
}

// ---- Grouping ----

interface Group {
  key: string;
  label: string;
  items: RemovedPicture[];
  /** For a group of repeats: the kept picture they repeat. */
  keptFile?: string;
}

/** "Not useful" groups first, in the mockup's order; then repeats, each led by its kept picture. */
export function groupRemoved(removed: RemovedPicture[]): { notUseful: Group[]; repeats: Group[] } {
  const notUseful = NOT_USEFUL
    .map(({ reason, label }) => ({ key: reason, label, items: removed.filter((r) => r.reason === reason) }))
    .filter((g) => g.items.length > 0);
  const byKept = new Map<string, RemovedPicture[]>();
  for (const r of removed) {
    if (r.reason !== 'repeat') continue;
    const k = r.repeatOf ?? '';
    byKept.set(k, [...(byKept.get(k) ?? []), r]);
  }
  const repeats = [...byKept.entries()]
    .map(([kept, items]) => ({ key: `repeat-${kept}`, label: 'Repeats', items, keptFile: kept || undefined }))
    .sort((a, b) => a.items[0].time.localeCompare(b.items[0].time));
  return { notUseful, repeats };
}

const hhmm = (t: string) => t.slice(0, 5);

/** "11:47:32" from "teams-frames/teams-20261118-114732-000.jpg". */
function timeOfFile(file: string | null): string | null {
  const m = file?.match(/-(\d{2})(\d{2})(\d{2})-\d+\.jpg$/i);
  return m ? `${m[1]}:${m[2]}:${m[3]}` : null;
}

const plural = (n: number) => `${n} ${n === 1 ? 'picture' : 'pictures'}`;

/** The summary line with its picture count in bold. */
function LineText({ line }: { line: string }) {
  const m = line.match(/^(.*?)(\d[\d,]* pictures?)(.*)$/);
  if (!m) return <>{line}</>;
  return <>{m[1]}<strong className="font-semibold text-gray-900">{m[2]}</strong>{m[3]}</>;
}

// ---- The header line, the inline panel, the viewer ----

export function PictureCleanup({
  meetingId, hasCleanup, version, onChanged,
}: {
  meetingId: string;
  /** The saved numbers say a cleanup record exists, so the page asks for it. */
  hasCleanup?: boolean;
  /** Bumped by the header when something outside this component may have changed the record. */
  version: number;
  /** Tells the header the picture counts changed, so "What was saved" reloads. */
  onChanged: () => void;
}) {
  const [loaded, setLoaded] = useState<{ id: string; view: CleanupView | null } | null>(null);
  const [open, setOpen] = useState(false);
  const [viewing, setViewing] = useState<number | null>(null);
  const [retrying, setRetrying] = useState(false);

  const load = useCallback(() => {
    invoke<CleanupView | null>('picture_cleanup_get', { meetingId })
      .then((v) => setLoaded({ id: meetingId, view: isCleanupView(v) ? v : null }))
      .catch(() => {});
  }, [meetingId]);

  useEffect(() => { if (hasCleanup || version > 0) load(); }, [load, hasCleanup, version]);
  useEffect(() => { setOpen(false); setViewing(null); setRetrying(false); }, [meetingId]);

  // A cleanup that finishes beside the note refreshes the page.
  useEffect(() => {
    let off: (() => void) | undefined;
    let dead = false;
    Promise.resolve()
      .then(() => listen(CLEANUP_EVENT, () => { setRetrying(false); onChanged(); }))
      .then((f) => { if (dead) f(); else off = f; })
      .catch(() => {});
    return () => { dead = true; off?.(); };
  }, [onChanged]);

  const view = loaded?.id === meetingId ? loaded.view : null;
  const groups = useMemo(() => groupRemoved(view?.removed ?? []), [view]);
  const ordered = useMemo(() => [...groups.notUseful.flatMap((g) => g.items), ...groups.repeats.flatMap((g) => g.items)], [groups]);

  const restore = useCallback(async (files: string[], message: string) => {
    if (files.length === 0) return;
    try {
      const next = await invoke<CleanupView>('picture_cleanup_restore', { meetingId, files });
      if (isCleanupView(next)) setLoaded({ id: meetingId, view: next });
      onChanged();
      toast.success(message, {
        action: {
          label: 'Undo',
          onClick: () => {
            invoke<CleanupView>('picture_cleanup_undo_restore', { meetingId, files })
              .then((v) => { if (isCleanupView(v)) setLoaded({ id: meetingId, view: v }); onChanged(); })
              .catch((e) => toast.error(`Could not undo: ${e}`));
          },
        },
      });
    } catch (e) {
      toast.error(`Could not restore: ${e}`);
    }
  }, [meetingId, onChanged]);

  const tryAgain = useCallback(() => {
    setRetrying(true);
    invoke('picture_cleanup_retry', { meetingId }).catch((e) => {
      setRetrying(false);
      toast.error(`${e}`);
    });
  }, [meetingId]);

  if (!view) return null;
  const { summary } = view;

  if (summary.failed) {
    return (
      <p className="text-[13px] text-gray-700">
        {view.running || retrying ? 'Trying picture cleanup again.' : (
          <>
            Picture cleanup did not finish.{' '}
            {view.canRetry && (
              <button type="button" onClick={tryAgain} className="font-medium text-blue-600 hover:underline">Try again</button>
            )}
          </>
        )}
      </p>
    );
  }
  if (!summary.line || summary.removed === 0) return null;

  const pending = ordered.filter((r) => !r.restored);
  const restoreMessage = (r: RemovedPicture) => `Restored the ${hhmm(r.time)} picture.`;
  const thumbs = pending.slice(0, 3);

  return (
    <>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 pt-1 text-[13px] text-gray-600">
        <span className="flex items-center gap-1" aria-hidden="true">
          {thumbs.map((r) => (
            <Picture key={r.file} meetingId={meetingId} file={r.removedFile} alt="" className="h-5 w-8 rounded-sm border border-gray-200 object-cover" />
          ))}
        </span>
        <span><LineText line={summary.line} /></span>
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          className="inline-flex items-center gap-1 font-medium text-blue-600 hover:underline"
        >
          {open ? 'Hide' : 'See what was removed'}
          {open ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
        </button>
        {view.bringBack && <span>{view.bringBack}</span>}
      </div>

      {open && (
        <section aria-label="Removed pictures" className="mt-2 max-h-[60vh] overflow-y-auto border-t border-gray-200 pt-3">
          <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2 pb-3">
            <div className="min-w-0">
              <h2 className="text-base font-semibold text-gray-900">{view.title}</h2>
              <p className="text-[13px] text-gray-600">{view.note}</p>
            </div>
            <Button
              variant="outline"
              size="sm"
              className="flex-shrink-0 text-[13px]"
              onClick={() => restore(pending.map((r) => r.file), `Restored all ${plural(pending.length)}.`)}
            >
              <RotateCcw /> Restore all {summary.removed}
            </Button>
          </div>

          {groups.notUseful.length > 0 && (
            <div>
              <h3 className="border-b border-gray-200 pb-1 text-[13px] font-semibold text-gray-900">
                Not useful <span className="font-normal text-gray-500">{plural(summary.notUseful)}</span>
              </h3>
              {groups.notUseful.map((g) => (
                <GroupRow key={g.key} group={g} meetingId={meetingId} ordered={ordered} onView={setViewing} onRestore={restore} />
              ))}
            </div>
          )}
          {groups.repeats.length > 0 && (
            <div className="pt-3">
              <h3 className="border-b border-gray-200 pb-1 text-[13px] font-semibold text-gray-900">
                Repeats <span className="font-normal text-gray-500">{plural(summary.repeats)}</span>
              </h3>
              {groups.repeats.map((g) => (
                <GroupRow key={g.key} group={g} meetingId={meetingId} ordered={ordered} onView={setViewing} onRestore={restore} />
              ))}
            </div>
          )}
        </section>
      )}

      <Viewer
        meetingId={meetingId}
        items={ordered}
        index={viewing}
        until={view.until}
        onIndex={setViewing}
        onRestore={(r) => restore([r.file], restoreMessage(r))}
      />
    </>
  );
}

function GroupRow({
  group, meetingId, ordered, onView, onRestore,
}: {
  group: Group;
  meetingId: string;
  ordered: RemovedPicture[];
  onView: (index: number) => void;
  onRestore: (files: string[], message: string) => void;
}) {
  const left = group.items.filter((r) => !r.restored);
  const restored = group.items.length - left.length;
  const keptTime = timeOfFile(group.keptFile ?? null);
  return (
    <div className="grid grid-cols-1 gap-3 border-b border-gray-100 py-3 sm:grid-cols-[180px_1fr]">
      <div>
        <p className="text-[13px] font-semibold text-gray-900">
          {group.keptFile ? `Repeats of the ${keptTime ? hhmm(keptTime) : ''} picture` : group.label}
        </p>
        <p className="text-[13px] text-gray-500">
          {plural(group.items.length)}{restored > 0 ? `, ${restored} restored` : ''}
        </p>
        {left.length > 0 && (
          <button
            type="button"
            onClick={() => onRestore(left.map((r) => r.file), `Restored ${plural(left.length)}.`)}
            className="mt-1 inline-flex items-center gap-1 text-[13px] font-medium text-blue-600 hover:underline"
          >
            <RotateCcw className="h-4 w-4" /> Restore these {left.length}
          </button>
        )}
      </div>
      <div className="flex flex-wrap gap-3">
        {group.keptFile && (
          <figure className="w-[150px]">
            <div className="relative">
              <Picture meetingId={meetingId} file={group.keptFile} alt={`Kept picture at ${keptTime ?? ''}`} className="aspect-video w-full rounded-md border-2 border-green-600 object-cover" />
              <span className="absolute left-1 top-1 inline-flex items-center gap-1 rounded-full bg-green-700 px-2 py-0.5 text-[12.5px] font-medium text-white">
                <Check className="h-3 w-3" /> Kept
              </span>
            </div>
            <figcaption className="text-[13px] text-gray-600">{keptTime ? hhmm(keptTime) : ''}</figcaption>
          </figure>
        )}
        {group.items.map((r) => (
          <figure key={r.file} className="w-[150px]">
            <div className="relative">
              <button
                type="button"
                onClick={() => onView(ordered.indexOf(r))}
                aria-label={`See the ${r.time} picture full size`}
                className="block w-full rounded-md focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-600"
              >
                <Picture
                  meetingId={meetingId}
                  file={r.restored ? r.file : r.removedFile}
                  alt={`Removed picture at ${r.time}`}
                  className={`aspect-video w-full rounded-md object-cover ${r.restored ? 'border-2 border-green-600' : 'border border-gray-300'}`}
                />
              </button>
              {r.restored ? (
                <span className="absolute left-1 top-1 inline-flex items-center gap-1 rounded-full bg-green-700 px-2 py-0.5 text-[12.5px] font-medium text-white">
                  <Check className="h-3 w-3" /> Restored
                </span>
              ) : (
                <button
                  type="button"
                  onClick={() => onRestore([r.file], `Restored the ${hhmm(r.time)} picture.`)}
                  aria-label={`Restore the ${r.time} picture`}
                  className="absolute bottom-1 right-1 rounded-md border border-gray-300 bg-white p-1 text-gray-700 shadow-sm hover:bg-gray-100"
                >
                  <RotateCcw className="h-4 w-4" />
                </button>
              )}
            </div>
            <figcaption className="text-[13px] text-gray-600">
              {hhmm(r.time)}{r.caption ? ` ${r.caption}` : ''}
            </figcaption>
          </figure>
        ))}
      </div>
    </div>
  );
}

function Viewer({
  meetingId, items, index, until, onIndex, onRestore,
}: {
  meetingId: string;
  items: RemovedPicture[];
  index: number | null;
  until: string | null;
  onIndex: (i: number | null) => void;
  onRestore: (r: RemovedPicture) => void;
}) {
  const current = index !== null ? items[index] : undefined;

  // Arrow keys step through the pictures in the order the panel shows them.
  useEffect(() => {
    if (index === null || items.length === 0) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'ArrowRight') onIndex((index + 1) % items.length);
      else if (e.key === 'ArrowLeft') onIndex((index - 1 + items.length) % items.length);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [index, items.length, onIndex]);

  const keptTime = timeOfFile(current?.repeatOf ?? null);
  const title = !current
    ? 'Removed picture'
    : current.reason === 'repeat'
      ? `Repeat of the ${keptTime ?? ''} picture`
      : current.caption ?? NOT_USEFUL.find((n) => n.reason === current.reason)?.label ?? 'Removed picture';

  return (
    <Dialog open={current !== undefined} onOpenChange={(o) => { if (!o) onIndex(null); }}>
      <DialogContent className="max-h-[92vh] w-[min(1400px,95vw)] max-w-none overflow-y-auto">
        <div className="flex items-start justify-between gap-3 pr-8">
          <div className="min-w-0">
            <DialogTitle className="text-base font-semibold text-gray-900">{title}</DialogTitle>
            <DialogDescription className="text-[13px] text-gray-600">
              {current && (current.restored
                ? `Restored. It is back in the recording folder.`
                : `Removed at ${current.time}.${until ? ` Deleted for good on ${until} unless you restore it.` : ''}`)}
            </DialogDescription>
          </div>
          <div className="flex flex-shrink-0 items-center gap-2">
            <Button variant="outline" size="icon" aria-label="Previous picture" onClick={() => index !== null && onIndex((index - 1 + items.length) % items.length)}>
              <ChevronLeft />
            </Button>
            <Button variant="outline" size="icon" aria-label="Next picture" onClick={() => index !== null && onIndex((index + 1) % items.length)}>
              <ChevronRight />
            </Button>
            <Button variant="blue" disabled={!current || current.restored} onClick={() => current && onRestore(current)}>
              <RotateCcw /> {current?.restored ? 'Restored' : 'Restore'}
            </Button>
          </div>
        </div>
        {current && (
          <div className={`grid gap-4 ${current.repeatOf ? 'grid-cols-1 md:grid-cols-2' : 'grid-cols-1'}`}>
            <figure>
              <figcaption className="mb-1 flex items-center gap-2 text-[13px] text-gray-700">
                <span className="inline-flex items-center gap-1 rounded-full bg-gray-100 px-2 py-0.5 font-medium">
                  <Trash2 className="h-3 w-3" /> {current.restored ? 'Restored' : 'Removed'}
                </span>
                {current.time}
              </figcaption>
              <Picture meetingId={meetingId} file={current.restored ? current.file : current.removedFile} alt={`Picture at ${current.time}`} className="max-h-[68vh] w-full rounded-md border border-gray-200 object-contain" />
            </figure>
            {current.repeatOf && (
              <figure>
                <figcaption className="mb-1 flex items-center gap-2 text-[13px] text-gray-700">
                  <span className="inline-flex items-center gap-1 rounded-full bg-green-50 px-2 py-0.5 font-medium text-green-700">
                    <Check className="h-3 w-3" /> Kept
                  </span>
                  {keptTime}
                </figcaption>
                <Picture meetingId={meetingId} file={current.repeatOf} alt={`Kept picture at ${keptTime ?? ''}`} className="max-h-[68vh] w-full rounded-md border border-gray-200 object-contain" />
              </figure>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
