// The Rust core emits this when a picture cleanup run ends (cleanup_run.rs EVENT).
export const CLEANUP_EVENT = 'assistant-cleanup';

/** The shape of Tauri's `listen`: resolves to a function that removes the listener. */
export type Listen = (event: string, handler: () => void) => Promise<() => void>;

/**
 * Calls `onChange` each time a picture cleanup finishes, until the returned stop function runs.
 * Never throws, also when `listen` fails.
 */
export function onCleanupFinished(listen: Listen, onChange: () => void): () => void {
  throw new Error('not implemented');
}
