'use client';

import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Check, ChevronDown, ChevronUp, RotateCcw, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import type { MeetingPicture, PictureReason } from './allPictures';
import { AllPicturesPanel } from './AllPicturesPanel';
import { Picture, REASON_LABEL, Viewer, hhmm, plural, timeOfFile } from './pictureParts';

// Shapes of the picture_cleanup_* commands (src-tauri/src/assistant/cleanup_run.rs).
// The backend writes the wording of the line, the panel title and the dates; this file places it.
export interface RemovedPicture {
  file: string;
  removedFile: string;
  time: string;
  reason: PictureReason;
  caption: string | null;
  repeatOf: string | null;
  bytes: number;
  restored: boolean;
  /** When this picture is deleted for good (epoch ms). */
  deleteAfterMs: number;
}

export interface CleanupView {
  summary: {
    line: string | null; failed: boolean; removed: number; repeats: number; notUseful: number;
    byYou: number; bytes: number; deleteAfterMs: number;
  };
  removed: RemovedPicture[];
  canRetry: boolean;
  running: boolean;
  title: string | null;
  note: string | null;
  bringBack: string | null;
  until: string | null;
}

/** Which panel takes the page body: Claude's and Joseph's removed pictures, or every picture. */
export type PanelKind = 'removed' | 'all' | null;

const CLEANUP_EVENT = 'assistant-cleanup';

const NOT_USEFUL: PictureReason[] = ['loading', 'greyed', 'blank', 'popup', 'before_share'];

export function isCleanupView(v: unknown): v is CleanupView {
  const o = v as CleanupView | null;
  return !!o && typeof o === 'object' && !!o.summary && typeof o.summary === 'object' && Array.isArray(o.removed);
}

// ---- Grouping ----

interface Group {
  key: string;
  label: string;
  items: RemovedPicture[];
  /** For a group of repeats: the kept picture they repeat. */
  keptFile?: string;
}

/** "Removed by you" first, then "Not useful" groups in the mockup's order, then repeats, each led by its kept picture. */
export function groupRemoved(removed: RemovedPicture[]): { byYou: Group[]; notUseful: Group[]; repeats: Group[] } {
  const byYou = removed.some((r) => r.reason === 'by_you')
    ? [{ key: 'by_you', label: REASON_LABEL.by_you, items: removed.filter((r) => r.reason === 'by_you') }]
    : [];
  const notUseful = NOT_USEFUL
    .map((reason) => ({ key: reason, label: REASON_LABEL[reason], items: removed.filter((r) => r.reason === reason) }))
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
  return { byYou, notUseful, repeats };
}

/** A record entry as the shared viewer takes it. */
function asPicture(r: RemovedPicture): MeetingPicture {
  return {
    file: r.file,
    shownFile: r.restored ? r.file : r.removedFile,
    time: r.time,
    bytes: r.bytes,
    removed: !r.restored,
    reason: r.reason,
    caption: r.caption,
    repeatOf: r.repeatOf,
    deleteAfterMs: r.deleteAfterMs,
  };
}

/** The summary line with its picture count in bold. */
function LineText({ line }: { line: string }) {
  const m = line.match(/^(.*?)(\d[\d,]* pictures?)(.*)$/);
  if (!m) return <>{line}</>;
  return <>{m[1]}<strong className="font-semibold text-gray-900">{m[2]}</strong>{m[3]}</>;
}

// ---- The header line, the inline panels, the viewer ----

export function PictureCleanup({
  meetingId, hasCleanup, version, onChanged, panelHost, panel, onPanelChange, onPendingChange,
}: {
  meetingId: string;
  /** The saved numbers say a cleanup record exists, so the page asks for it. */
  hasCleanup?: boolean;
  /** Bumped by the header when something outside this component may have changed the record. */
  version: number;
  /** Tells the header the picture counts changed, so "What was saved" reloads. */
  onChanged: () => void;
  /** Where the open panel goes: the page body, which the header hides while a panel shows. */
  panelHost?: HTMLElement | null;
  /** The panel that is open; the header owns it so only one is open at a time. */
  panel: PanelKind;
  onPanelChange: (panel: PanelKind) => void;
  /** Reports how many pictures are removed and not restored (Claude's and Joseph's). */
  onPendingChange?: (pending: number) => void;
}) {
  const [loaded, setLoaded] = useState<{ id: string; view: CleanupView | null } | null>(null);
  const [viewing, setViewing] = useState<number | null>(null);
  const [retrying, setRetrying] = useState(false);
  // Bumped after every Remove, Restore and Undo: the All pictures panel reads its list again.
  const [tick, setTick] = useState(0);

  const load = useCallback(() => {
    invoke<CleanupView | null>('picture_cleanup_get', { meetingId })
      .then((v) => setLoaded({ id: meetingId, view: isCleanupView(v) ? v : null }))
      .catch(() => {});
  }, [meetingId]);

  useEffect(() => { if (hasCleanup || version > 0) load(); }, [load, hasCleanup, version]);
  useEffect(() => { setViewing(null); setRetrying(false); }, [meetingId]);

  // A cleanup that finishes beside the note refreshes the page.
  useEffect(() => {
    let off: (() => void) | undefined;
    let dead = false;
    Promise.resolve()
      .then(() => listen(CLEANUP_EVENT, () => { setRetrying(false); setTick((n) => n + 1); onChanged(); }))
      .then((f) => { if (dead) f(); else off = f; })
      .catch(() => {});
    return () => { dead = true; off?.(); };
  }, [onChanged]);

  const view = loaded?.id === meetingId ? loaded.view : null;
  const groups = useMemo(() => groupRemoved(view?.removed ?? []), [view]);
  const ordered = useMemo(
    () => [...groups.byYou, ...groups.notUseful, ...groups.repeats].flatMap((g) => g.items),
    [groups],
  );
  const items = useMemo(() => ordered.map(asPicture), [ordered]);
  const pending = ordered.filter((r) => !r.restored);
  const byYouPaths = useMemo(
    () => new Map(pending.filter((r) => r.reason === 'by_you').map((r) => [r.file, r.removedFile] as const)),
    [pending],
  );

  const showing = panel === 'removed' && !!view && pending.length > 0;
  // Everything restored while the removed panel is open: hand the page body back.
  useEffect(() => { if (panel === 'removed' && view && pending.length === 0) onPanelChange(null); }, [panel, view, pending.length, onPanelChange]);
  useEffect(() => { onPendingChange?.(pending.length); }, [pending.length, onPendingChange]);

  /** Redraw from the command's returned view, never from an edited copy. */
  const apply = useCallback((next: unknown) => {
    if (isCleanupView(next)) setLoaded({ id: meetingId, view: next });
    setTick((n) => n + 1);
    onChanged();
  }, [meetingId, onChanged]);

  const restore = useCallback(async (files: string[], message: string) => {
    if (files.length === 0) return;
    try {
      apply(await invoke<CleanupView>('picture_cleanup_restore', { meetingId, files }));
      toast.success(message, {
        action: {
          label: 'Undo',
          onClick: () => {
            invoke<CleanupView>('picture_cleanup_undo_restore', { meetingId, files })
              .then(apply)
              .catch((e) => toast.error(`Could not undo: ${e}`));
          },
        },
      });
    } catch (e) {
      toast.error(`Could not restore: ${e}`);
    }
  }, [meetingId, apply]);

  const removeByYou = useCallback(async (files: string[], message: string) => {
    if (files.length === 0) return;
    try {
      apply(await invoke<CleanupView>('picture_cleanup_remove', { meetingId, files }));
      toast.success(message, {
        action: {
          label: 'Undo',
          onClick: () => {
            invoke<CleanupView>('picture_cleanup_restore', { meetingId, files })
              .then(apply)
              .catch((e) => toast.error(`Could not undo: ${e}`));
          },
        },
      });
    } catch (e) {
      toast.error(`Could not remove: ${e}`);
    }
  }, [meetingId, apply]);

  const tryAgain = useCallback(() => {
    setRetrying(true);
    invoke('picture_cleanup_retry', { meetingId }).catch((e) => {
      setRetrying(false);
      toast.error(`${e}`);
    });
  }, [meetingId]);

  const summary = view?.summary;
  const thumbs = pending.slice(0, 3);

  let text: ReactNode = null;
  if (summary?.failed) {
    text = view?.running || retrying ? 'Trying picture cleanup again.' : (
      <>
        Picture cleanup did not finish.{' '}
        {view?.canRetry && (
          <button type="button" onClick={tryAgain} className="font-medium text-blue-600 hover:underline">Try again</button>
        )}
      </>
    );
  } else if (summary?.line) {
    text = <LineText line={summary.line} />;
  }

  return (
    <>
      {view && summary && text && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 pt-1 text-[13px] text-gray-600">
          {pending.length > 0 && (
            <span className="flex items-center gap-1" aria-hidden="true">
              {thumbs.map((r) => (
                <Picture key={r.file} meetingId={meetingId} file={r.removedFile} alt="" className="h-5 w-8 rounded-sm border border-gray-200 object-cover" />
              ))}
            </span>
          )}
          <span className="min-w-0 break-words">{text}</span>
          {pending.length > 0 && (
            <button
              type="button"
              onClick={() => onPanelChange(panel === 'removed' ? null : 'removed')}
              aria-expanded={panel === 'removed'}
              className="inline-flex items-center gap-1 font-medium text-blue-600 hover:underline"
            >
              {panel === 'removed' ? 'Hide' : 'See what was removed'}
              {panel === 'removed' ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </button>
          )}
          {pending.length > 0 && view.bringBack && <span>{view.bringBack}</span>}
        </div>
      )}

      {showing && view && summary && panelHost && createPortal(
        <section aria-label="Removed pictures" className="px-4 pb-6 pt-3">
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
              <RotateCcw /> Restore all {pending.length}
            </Button>
          </div>

          {groups.byYou.length > 0 && (
            <div>
              <h3 className="border-b border-gray-200 pb-1 text-[13px] font-semibold text-gray-900">
                Removed by you <span className="font-normal text-gray-500">{plural(summary.byYou)}</span>
              </h3>
              {groups.byYou.map((g) => (
                <GroupRow key={g.key} group={g} meetingId={meetingId} ordered={ordered} byYouPaths={byYouPaths} onView={setViewing} onRestore={restore} />
              ))}
            </div>
          )}
          {groups.notUseful.length > 0 && (
            <div className={groups.byYou.length > 0 ? 'pt-3' : ''}>
              <h3 className="border-b border-gray-200 pb-1 text-[13px] font-semibold text-gray-900">
                Not useful <span className="font-normal text-gray-500">{plural(summary.notUseful)}</span>
              </h3>
              {groups.notUseful.map((g) => (
                <GroupRow key={g.key} group={g} meetingId={meetingId} ordered={ordered} byYouPaths={byYouPaths} onView={setViewing} onRestore={restore} />
              ))}
            </div>
          )}
          {groups.repeats.length > 0 && (
            <div className="pt-3">
              <h3 className="border-b border-gray-200 pb-1 text-[13px] font-semibold text-gray-900">
                Repeats <span className="font-normal text-gray-500">{plural(summary.repeats)}</span>
              </h3>
              {groups.repeats.map((g) => (
                <GroupRow key={g.key} group={g} meetingId={meetingId} ordered={ordered} byYouPaths={byYouPaths} onView={setViewing} onRestore={restore} />
              ))}
            </div>
          )}
        </section>,
        panelHost,
      )}

      {panel === 'all' && panelHost && createPortal(
        <AllPicturesPanel meetingId={meetingId} tick={tick} onRemove={removeByYou} onRestore={restore} />,
        panelHost,
      )}

      <Viewer
        meetingId={meetingId}
        items={showing ? items : []}
        index={showing ? viewing : null}
        onIndex={setViewing}
        byYou={byYouPaths}
        onRestore={(p) => restore([p.file], `Restored the ${hhmm(p.time)} picture.`)}
      />
    </>
  );
}

function GroupRow({
  group, meetingId, ordered, byYouPaths, onView, onRestore,
}: {
  group: Group;
  meetingId: string;
  ordered: RemovedPicture[];
  byYouPaths: Map<string, string>;
  onView: (index: number) => void;
  onRestore: (files: string[], message: string) => void;
}) {
  const left = group.items.filter((r) => !r.restored);
  const restored = group.items.length - left.length;
  const keptTime = timeOfFile(group.keptFile ?? null);
  const keptRemovedByYou = group.keptFile ? byYouPaths.get(group.keptFile) : undefined;
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
              <Picture
                meetingId={meetingId}
                file={keptRemovedByYou ?? group.keptFile}
                alt={`${keptRemovedByYou ? 'Removed' : 'Kept'} picture at ${keptTime ?? ''}`}
                className={`aspect-video w-full rounded-md object-cover ${keptRemovedByYou ? 'border border-gray-300' : 'border-2 border-green-600'}`}
              />
              {keptRemovedByYou ? (
                <span className="absolute left-1 top-1 inline-flex items-center gap-1 rounded-full bg-gray-800 px-2 py-0.5 text-[12.5px] font-medium text-white">
                  <Trash2 className="h-3 w-3" /> Removed by you
                </span>
              ) : (
                <span className="absolute left-1 top-1 inline-flex items-center gap-1 rounded-full bg-green-700 px-2 py-0.5 text-[12.5px] font-medium text-white">
                  <Check className="h-3 w-3" /> Kept
                </span>
              )}
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
