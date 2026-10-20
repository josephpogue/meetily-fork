'use client';

import { FileText, Pencil } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useAssistant } from '@/contexts/AssistantContext';
import { LANE_TONE } from './KindChip';
import { cn } from '@/lib/utils';

interface BriefBarProps {
  /** The full brief is showing in the Ask lane's column. */
  open: boolean;
  onToggleOpen: () => void;
  onEdit: () => void;
}

/**
 * The slim bar under the status line: a two-line preview of the meeting brief
 * (the one deliberate clamp in the panel) with Read all and Edit. While the
 * full brief is open the bar takes the Say-blue tint.
 */
export function BriefBar({ open, onToggleOpen, onEdit }: BriefBarProps) {
  const { briefDraft, briefSaved } = useAssistant();
  const text = briefDraft.trim();
  const dirty = briefDraft !== briefSaved;

  return (
    <div
      className={cn(
        'flex shrink-0 items-start gap-2.5 border-b px-3.5 py-2.5',
        open ? `${LANE_TONE.say.tint} ${LANE_TONE.say.line}` : 'border-gray-200 bg-gray-50'
      )}
    >
      <FileText className="mt-0.5 h-4 w-4 shrink-0 text-gray-400" aria-hidden="true" />
      <p className="line-clamp-2 min-w-0 flex-1 text-sm leading-snug text-gray-600">
        {text ? (
          <>
            <b className="font-semibold text-gray-900">Brief:</b> {text}
          </>
        ) : (
          <span className="text-gray-500">No brief yet.</span>
        )}
      </p>
      <div className="flex shrink-0 items-center gap-1.5">
        {dirty ? <span className="text-xs font-medium text-amber-600">Unsaved</span> : null}
        {text ? (
          <Button type="button" variant="outline" size="sm" className="h-7 bg-white px-2.5 text-xs" onClick={onToggleOpen}>
            {open ? 'Back to questions' : 'Read all'}
          </Button>
        ) : null}
        <Button type="button" variant="outline" size="sm" className="h-7 bg-white px-2.5 text-xs" onClick={onEdit}>
          {text ? (
            <>
              <Pencil className="h-3 w-3" aria-hidden="true" />
              Edit
            </>
          ) : (
            'Add brief'
          )}
        </Button>
      </div>
    </div>
  );
}
