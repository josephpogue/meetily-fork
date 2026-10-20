'use client';

/**
 * The Ask lane: open gap questions, For-you questions, then every answered
 * question with its answer. Placeholder until its slice lands.
 */
export function AskLane({ disabled }: { disabled: boolean }) {
  void disabled;
  return <section aria-label="Ask" className="flex min-h-0 min-w-0 flex-1 flex-col" />;
}
