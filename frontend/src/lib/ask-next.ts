/**
 * Pure helpers for the Ask next section of the assistant panel.
 */

/** A For-you answer becomes a new line at the end of the brief. */
export function appendBriefAnswer(brief: string, question: string, answer: string): string {
  const line = `- ${question.trim()} ${answer.trim()}`;
  const head = brief.trimEnd();
  return head ? `${head}\n${line}` : line;
}

export function formatUpdatedAgo(updatedAt: number | null, now: number): string {
  if (updatedAt === null) return '';
  const secs = Math.max(0, Math.round((now - updatedAt) / 1000));
  if (secs < 5) return 'updated just now';
  if (secs < 60) return `updated ${secs}s ago`;
  return `updated ${Math.floor(secs / 60)}m ago`;
}

export interface BriefState {
  /** What is in the box. */
  draft: string;
  /** The last brief known to be saved: the backend's copy, or what Save just sent. */
  saved: string;
}

/**
 * The backend's copy of the brief changed to `next`. With nothing unsaved,
 * adopt it. With unsaved edits, keep the draft; but when `next` only extends
 * the saved baseline (a For-you answer appended a line), add that line to the
 * end of the draft so a later Save does not erase it.
 */
export function reconcileBrief({ draft, saved }: BriefState, next: string): BriefState {
  if (next === saved) return { draft, saved };
  if (draft === saved) return { draft: next, saved: next };
  const base = saved.trimEnd();
  if (!next.startsWith(base)) return { draft, saved: next };
  const suffix = next.slice(base.length);
  if (!suffix) return { draft, saved: next };
  return { draft: draft.trimEnd() + (base ? '' : '\n') + suffix, saved: next };
}
