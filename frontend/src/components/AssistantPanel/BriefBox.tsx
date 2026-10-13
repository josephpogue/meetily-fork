'use client';

import { useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { useAssistant } from '@/contexts/AssistantContext';

/**
 * The meeting brief: optional, editable before and during the call, and
 * committed only by an explicit Save (the button or Cmd+Enter), never by
 * leaving the box. An "Unsaved" marker in the header row shows whenever the
 * text differs from the saved brief, even when the box is folded.
 * The draft and the saved baseline live in AssistantContext, so collapsing
 * the panel or leaving the page does not lose an unsaved edit. The backend's
 * copy (sent with every assistant-asks event) wins whenever it changes, unless
 * Joseph has unsaved edits: then his text stays, a For-you answer appended to
 * the brief is added to its end, and the marker stays on.
 */
export function BriefBox() {
  const { briefDraft: value, setBriefDraft: setValue, briefSaved, saveBrief: save } = useAssistant();
  const [expanded, setExpanded] = useState(true);

  const dirty = value !== briefSaved;

  return (
    <div className="border-b border-gray-200 p-3">
      <div className="mb-1 flex items-center gap-2">
        <button
          type="button"
          aria-expanded={expanded}
          onClick={() => setExpanded(!expanded)}
          className="flex flex-1 items-center gap-1 text-xs font-medium text-gray-500"
        >
          {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
          Meeting brief (optional)
        </button>
        {dirty ? <span className="text-xs font-medium text-amber-600">Unsaved</span> : null}
      </div>
      {expanded ? (
        <>
          <Textarea
            aria-label="Meeting brief (optional)"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && e.metaKey) {
                e.preventDefault();
                save();
              }
            }}
            placeholder="What is this meeting about?"
            className="min-h-[60px] text-sm"
          />
          <div className="mt-1 flex justify-end">
            <Button type="button" size="sm" disabled={!dirty} onClick={save}>
              Save
            </Button>
          </div>
        </>
      ) : (
        <p className="truncate text-xs text-gray-400">{value.trim() || 'No brief'}</p>
      )}
    </div>
  );
}
