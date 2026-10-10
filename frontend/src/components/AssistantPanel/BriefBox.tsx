'use client';

import { useEffect, useRef, useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { Textarea } from '@/components/ui/textarea';
import { useAssistant } from '@/contexts/AssistantContext';

/**
 * The meeting brief: optional, editable before and during the call, and
 * committed on blur. Folds to one line so it does not crowd the panel.
 * The backend's copy (sent with every assistant-asks event) wins whenever
 * it changes, unless Joseph is typing in the box at that moment.
 */
export function BriefBox() {
  const { asks, setBrief } = useAssistant();
  const [value, setValue] = useState(asks.brief);
  const [expanded, setExpanded] = useState(true);
  const focused = useRef(false);

  useEffect(() => {
    if (!focused.current) setValue(asks.brief);
  }, [asks.brief]);

  return (
    <div className="border-b border-gray-200 p-3">
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded(!expanded)}
        className="mb-1 flex w-full items-center gap-1 text-xs font-medium text-gray-500"
      >
        {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        Meeting brief (optional)
      </button>
      {expanded ? (
        <Textarea
          aria-label="Meeting brief (optional)"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onFocus={() => {
            focused.current = true;
          }}
          onBlur={() => {
            focused.current = false;
            setBrief(value);
          }}
          placeholder="What is this meeting about?"
          className="min-h-[60px] text-sm"
        />
      ) : (
        <p className="truncate text-xs text-gray-400">{value.trim() || 'No brief'}</p>
      )}
    </div>
  );
}
