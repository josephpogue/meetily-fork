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
