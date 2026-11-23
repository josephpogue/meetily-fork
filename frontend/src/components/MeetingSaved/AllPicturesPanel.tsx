'use client';

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { RotateCcw, Trash2 } from 'lucide-react';
import { shownPictures, type AllPictures, type MeetingPicture } from './allPictures';
import { Picture, REASON_LABEL, Viewer } from './pictureParts';

/**
 * Every picture of a meeting, kept and removed, in capture order. It takes the page body, like the
 * removed panel. It never edits its own list: `tick` changes after every Remove, Restore or Undo,
 * and the list is read again from picture_cleanup_all.
 */
export function AllPicturesPanel({
  meetingId, tick, onRemove, onRestore,
}: {
  meetingId: string;
  tick: number;
  onRemove: (files: string[], message: string) => void;
  onRestore: (files: string[], message: string) => void;
}) {
  const [all, setAll] = useState<{ id: string; data: AllPictures } | null>(null);
  const [showRemoved, setShowRemoved] = useState(false);
  const [viewing, setViewing] = useState<number | null>(null);

  useEffect(() => {
    let dead = false;
    invoke<AllPictures>('picture_cleanup_all', { meetingId })
      .then((data) => { if (!dead) setAll({ id: meetingId, data }); })
      .catch((e) => console.error('Failed to load the pictures:', e));
    return () => { dead = true; };
  }, [meetingId, tick]);

  const data = all?.id === meetingId ? all.data : null;
  const show = showRemoved && !!data && data.removed > 0;
  const shown = useMemo(() => (data ? shownPictures(data, show) : []), [data, show]);

  const remove = (p: MeetingPicture) => onRemove([p.file], `Removed the ${p.time} picture.`);
  const restore = (p: MeetingPicture) => onRestore([p.file], `Restored the ${p.time} picture.`);

  if (!data) return <section aria-label="All pictures" className="px-4 pb-6 pt-3 text-[13px] text-gray-600">Loading the pictures.</section>;

  return (
    <section aria-label="All pictures" className="px-4 pb-6 pt-3">
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2 pb-3">
        <div className="min-w-0">
          <h2 className="text-base font-semibold text-gray-900">{data.title}</h2>
          <p className="text-[13px] text-gray-600">
            Click a picture to see it full size. Remove one Claude kept by mistake; you can bring it back for five days.
          </p>
        </div>
        {data.removed > 0 && (
          <button
            type="button"
            role="switch"
            aria-checked={show}
            onClick={() => setShowRemoved((v) => !v)}
            className="inline-flex flex-shrink-0 items-center gap-2 rounded-md text-[13px] font-medium text-gray-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-600"
          >
            <span aria-hidden="true" className={`relative inline-block h-5 w-9 rounded-full ${show ? 'bg-blue-600' : 'bg-gray-400'}`}>
              <span className={`absolute top-0.5 h-4 w-4 rounded-full bg-white transition-all ${show ? 'left-[18px]' : 'left-0.5'}`} />
            </span>
            Show removed ({data.removed})
          </button>
        )}
      </div>

      {shown.length === 0 ? (
        <p className="text-[13px] text-gray-600">There are no pictures to show.</p>
      ) : (
        <div className="flex flex-wrap gap-3">
          {shown.map((p, i) => (
            <figure key={p.shownFile} className="w-[150px]">
              <div className="relative">
                <button
                  type="button"
                  onClick={() => setViewing(i)}
                  aria-label={`See the ${p.time} picture full size`}
                  className="block w-full rounded-md focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-600"
                >
                  <Picture
                    meetingId={meetingId}
                    file={p.shownFile}
                    alt={`${p.removed ? 'Removed picture' : 'Picture'} at ${p.time}`}
                    className={`aspect-video w-full rounded-md border border-gray-300 object-cover ${p.removed ? 'opacity-50' : ''}`}
                  />
                </button>
                {p.removed && p.reason && (
                  <span className="absolute left-1 top-1 inline-flex max-w-[calc(100%-8px)] items-center gap-1 rounded-full bg-gray-800 px-2 py-0.5 text-[12.5px] font-medium text-white">
                    <Trash2 className="h-3 w-3 flex-shrink-0" /> <span className="truncate">{REASON_LABEL[p.reason]}</span>
                  </span>
                )}
                <button
                  type="button"
                  onClick={() => (p.removed ? restore(p) : remove(p))}
                  aria-label={`${p.removed ? 'Restore' : 'Remove'} the ${p.time} picture`}
                  title={p.removed ? 'Restore' : 'Remove'}
                  className="absolute bottom-1 right-1 rounded-md border border-gray-300 bg-white p-1 text-gray-700 shadow-sm hover:bg-gray-100"
                >
                  {p.removed ? <RotateCcw className="h-4 w-4" /> : <Trash2 className="h-4 w-4" />}
                </button>
              </div>
              <figcaption className="text-[13px] text-gray-600">
                {p.time}{p.caption ? ` ${p.caption}` : ''}
              </figcaption>
            </figure>
          ))}
        </div>
      )}

      <Viewer
        meetingId={meetingId}
        items={shown}
        index={viewing}
        onIndex={setViewing}
        onRestore={restore}
        onRemove={remove}
      />
    </section>
  );
}
