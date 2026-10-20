'use client';

import { Pencil } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useAssistant } from '@/contexts/AssistantContext';
import { BriefBox } from './BriefBox';

interface BriefFullProps {
  editing: boolean;
  onEdit: () => void;
  /** A save happened: go back to the read view. */
  onSaved: () => void;
}

/**
 * The whole meeting brief, in the Ask lane's column while the brief is open.
 * Line breaks are kept and nothing is clamped. Edit swaps in the editor.
 */
export function BriefFull({ editing, onEdit, onSaved }: BriefFullProps) {
  const { briefDraft, briefSaved } = useAssistant();
  const dirty = briefDraft !== briefSaved;

  return (
    <section aria-label="Meeting brief" className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center justify-between gap-2 px-3.5 pb-2 pt-3">
        <h3 className="text-sm font-semibold text-gray-900">Meeting brief</h3>
        {!editing ? (
          <div className="flex items-center gap-2">
            {dirty ? <span className="text-xs font-medium text-amber-600">Unsaved</span> : null}
            <Button type="button" variant="outline" size="sm" className="h-7 px-2.5 text-xs" onClick={onEdit}>
              <Pencil className="h-3 w-3" aria-hidden="true" />
              Edit
            </Button>
          </div>
        ) : null}
      </div>
      <div tabIndex={0} className="min-h-0 flex-1 overflow-y-auto px-3.5 pb-4 outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-blue-400">
        {editing ? (
          <BriefBox onSaved={onSaved} />
        ) : briefDraft.trim() ? (
          <p className="whitespace-pre-wrap break-words text-sm leading-relaxed text-gray-700">{briefDraft}</p>
        ) : (
          <p className="text-sm text-gray-500">No brief yet.</p>
        )}
      </div>
    </section>
  );
}
