'use client';

import { useEffect, useRef, useState } from 'react';
import ReactMarkdown, { Components } from 'react-markdown';
import { Check, ChevronsUpDown, FileText, FolderOpen, Loader2, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ScrollArea } from '@/components/ui/scroll-area';
import { useAssistant } from '@/contexts/AssistantContext';

// Styled explicitly (not via the Tailwind typography plugin, whose availability
// is ambiguous here: the repo carries both tailwind.config.js and .ts, and only
// the .ts one lists the plugin).
const markdownComponents: Components = {
  h1: ({ children }) => <h1 className="mb-1 mt-3 text-base font-semibold text-gray-900 first:mt-0">{children}</h1>,
  h2: ({ children }) => <h2 className="mb-1 mt-3 text-sm font-semibold text-gray-900 first:mt-0">{children}</h2>,
  h3: ({ children }) => <h3 className="mb-1 mt-2 text-sm font-semibold text-gray-800">{children}</h3>,
  p: ({ children }) => <p className="mb-2 text-sm text-gray-700">{children}</p>,
  ul: ({ children }) => <ul className="mb-2 ml-4 list-disc space-y-0.5 text-sm text-gray-700">{children}</ul>,
  ol: ({ children }) => <ol className="mb-2 ml-4 list-decimal space-y-0.5 text-sm text-gray-700">{children}</ol>,
  li: ({ children }) => <li>{children}</li>,
  strong: ({ children }) => <strong className="font-semibold text-gray-900">{children}</strong>,
  a: ({ children, href }) => (
    <a href={href} className="text-blue-600 underline" target="_blank" rel="noreferrer">
      {children}
    </a>
  ),
};

/**
 * End-of-meeting note flow: Draft note -> preview -> Save / Discard.
 * Save is the only write; nothing else in the panel writes anywhere.
 */
export function NoteBar() {
  const {
    note,
    draftNote,
    saveNote,
    savedNoteProject,
    saveNoteError,
    listVaultProjects,
    suggestVaultProject,
    discardNote,
  } = useAssistant();
  const [projects, setProjects] = useState<string[]>([]);
  const [suggested, setSuggested] = useState<string | null>(null);
  // '' is "No project". Set to the suggestion when the list loads, unless Joseph
  // already picked for this note: his pick survives a background redraft (a
  // delete after Draft note) and resets only when the note goes back to idle.
  const [choice, setChoice] = useState('');
  const picked = useRef(false);
  const [loaded, setLoaded] = useState(false);
  const [listFailed, setListFailed] = useState(false);

  const ready = note.state === 'ready';
  const idle = note.state === 'idle';
  useEffect(() => {
    if (!idle) return;
    picked.current = false;
    setChoice('');
  }, [idle]);

  useEffect(() => {
    if (!ready) return;
    let cancelled = false;
    setLoaded(false);
    Promise.all([listVaultProjects(), suggestVaultProject()]).then(([list, suggestion]) => {
      if (cancelled) return;
      const all = list ?? [];
      const pick = suggestion && all.includes(suggestion) ? suggestion : null;
      setProjects(all);
      setListFailed(list === null);
      setSuggested(pick);
      setChoice((current) => (picked.current && (current === '' || all.includes(current)) ? current : (pick ?? '')));
      setLoaded(true);
    });
    return () => {
      cancelled = true;
    };
  }, [ready, listVaultProjects, suggestVaultProject]);

  const others = projects.filter((slug) => slug !== suggested);
  const hint = listFailed
    ? 'Could not read the vault projects. Save keeps the note without a project.'
    : loaded && projects.length === 0
      ? 'No vault projects found. Save keeps the note without a project.'
      : choice === ''
        ? 'Save keeps the note without a project.'
        : choice === suggested
          ? 'Suggested from the brief and the call. Save files the note under this project.'
          : 'Save files the note under this project.';

  return (
    <div className="border-t border-gray-200 bg-gray-50 p-3">
      {note.state === 'idle' && (
        <Button variant="outline" size="sm" className="w-full" onClick={draftNote}>
          <FileText className="h-4 w-4" />
          Draft note
        </Button>
      )}

      {note.state === 'drafting' && (
        <div className="flex items-center justify-center gap-2 py-2 text-sm text-gray-500">
          <Loader2 className="h-4 w-4 animate-spin" />
          Drafting note...
        </div>
      )}

      {note.state === 'ready' && (
        <div className="space-y-2">
          <ScrollArea className="h-40 rounded-md border border-gray-200 bg-white p-3">
            <ReactMarkdown components={markdownComponents}>{note.markdown}</ReactMarkdown>
          </ScrollArea>
          <div>
            <div className="flex items-center gap-2">
              <label
                htmlFor="note-project"
                className="flex shrink-0 items-center gap-1.5 text-[13px] font-semibold text-gray-700"
              >
                <FolderOpen className="h-4 w-4 text-gray-400" />
                Project
              </label>
              <div className="relative min-w-0 flex-1">
                <select
                  id="note-project"
                  aria-label="Vault project"
                  value={choice}
                  onChange={(e) => {
                    picked.current = true;
                    setChoice(e.target.value);
                  }}
                  className="h-8 w-full min-w-0 cursor-pointer appearance-none rounded-md border border-gray-300 bg-white pl-2.5 pr-8 text-[13px] text-gray-900 hover:border-gray-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
                >
                  {suggested && (
                    <optgroup label="Suggested">
                      <option value={suggested}>{`${suggested} (suggested)`}</option>
                    </optgroup>
                  )}
                  <option value="">No project</option>
                  {others.length > 0 && (
                    <optgroup label="All projects">
                      {others.map((slug) => (
                        <option key={slug} value={slug}>
                          {slug}
                        </option>
                      ))}
                    </optgroup>
                  )}
                </select>
                <ChevronsUpDown className="pointer-events-none absolute right-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-gray-400" />
              </div>
            </div>
            <p className="mt-1.5 text-[12.5px] leading-snug text-gray-500">{hint}</p>
          </div>
          <div className="flex gap-2">
            <Button
              variant="blue"
              size="sm"
              className="flex-1"
              disabled={!loaded}
              onClick={() => saveNote(choice === '' ? null : choice)}
            >
              <Check className="h-4 w-4" />
              Save
            </Button>
            <Button variant="outline" size="sm" className="flex-1" onClick={discardNote}>
              <X className="h-4 w-4" />
              Discard
            </Button>
          </div>
          {saveNoteError && <p className="text-sm text-red-600">{`Could not save the note: ${saveNoteError}`}</p>}
        </div>
      )}

      {note.state === 'saved' && (
        <p role="status" className="flex items-center justify-center gap-1.5 text-sm text-green-700">
          <Check className="h-4 w-4" />
          {typeof savedNoteProject === 'string' ? (
            <span>
              Note saved to <b className="font-semibold">{savedNoteProject}</b>.
            </span>
          ) : savedNoteProject === null ? (
            <span>Note saved, no project.</span>
          ) : (
            <span>Note saved.</span>
          )}
        </p>
      )}

      {note.state === 'failed' && (
        <div className="space-y-2">
          <p className="text-sm text-red-600">{note.error || 'Note drafting failed.'}</p>
          <Button variant="outline" size="sm" className="w-full" onClick={draftNote}>
            Try again
          </Button>
        </div>
      )}
    </div>
  );
}
