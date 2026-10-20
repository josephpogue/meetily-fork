'use client';

import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { useAssistant } from '@/contexts/AssistantContext';

/**
 * The meeting brief editor: optional, editable before and during the call, and
 * committed only by an explicit Save (the button or Cmd+Enter), never by
 * leaving the box. An "Unsaved" marker beside Save shows whenever the text
 * differs from the saved brief. BriefFull shows it in the Ask lane's column;
 * onSaved lets that view go back to reading once a save has happened.
 * The draft and the saved baseline live in AssistantContext, so collapsing
 * the panel or leaving the page does not lose an unsaved edit. The backend's
 * copy (sent with every assistant-asks event) wins whenever it changes, unless
 * Joseph has unsaved edits: then his text stays, a For-you answer appended to
 * the brief is added to its end, and the marker stays on.
 */
export function BriefBox({ onSaved }: { onSaved?: () => void }) {
  const { briefDraft: value, setBriefDraft: setValue, briefSaved, saveBrief } = useAssistant();

  const dirty = value !== briefSaved;

  const save = () => {
    saveBrief();
    onSaved?.();
  };

  return (
    <div>
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
        className="min-h-[160px] text-sm"
      />
      <div className="mt-2 flex items-center justify-end gap-3">
        {dirty ? <span className="text-xs font-medium text-amber-600">Unsaved</span> : null}
        <Button type="button" size="sm" disabled={!dirty} onClick={save}>
          Save
        </Button>
      </div>
    </div>
  );
}
