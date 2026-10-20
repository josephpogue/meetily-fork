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
  // Only a whole new line counts as an append; any other edit keeps the draft.
  if (!suffix || (base && !suffix.startsWith('\n'))) return { draft, saved: next };
  return { draft: draft.trimEnd() + (base ? '' : '\n') + suffix, saved: next };
}

/** The stamp on the Open sub-header: "Checked 12s ago" after a deep check, else "Updated 12s ago". */
export function formatOpenStamp(updatedAt: number | null, now: number, checked: boolean): string {
  const ago = formatUpdatedAgo(updatedAt, now);
  if (!ago) return '';
  const rest = ago.slice('updated '.length);
  return `${checked ? 'Checked' : 'Updated'} ${rest}`;
}

/** Ids on the new open list that were not on the one before. Empty without a previous list. */
export function newAskIds(previous: string[] | null, next: string[]): string[] {
  if (previous === null) return [];
  const seen = new Set(previous);
  return next.filter((id) => !seen.has(id));
}

/** The line under an answered question: how it got answered, and when. */
export function answeredHowLine(how: 'call' | 'chat' | 'you', at: number): string {
  const time = new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  if (how === 'call') return `On the call at ${time}`;
  if (how === 'chat') return `Worked out with you in Ask at ${time}`;
  return `You answered at ${time}`;
}
