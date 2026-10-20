/** Width rules for the assistant panel, which Joseph can drag wider or narrower. */

export const PANEL_DEFAULT_WIDTH = 600;
export const PANEL_MIN_WIDTH = 440;
/** The transcript always keeps at least this much of the row. */
export const TRANSCRIPT_MIN_WIDTH = 420;
export const PANEL_WIDTH_STORAGE_KEY = 'meetily-fork.assistant-panel-width';
/** Arrow keys on the drag handle move the edge by this much. */
export const PANEL_KEY_STEP = 16;

/** The widest the panel may be in a row of this width; never below the minimum. */
export function maxPanelWidth(rowWidth: number): number {
  return Math.max(PANEL_MIN_WIDTH, rowWidth - TRANSCRIPT_MIN_WIDTH);
}

/**
 * The width to show: the stored choice (or the default when there is none or
 * it is not a finite number), kept between the minimum and what leaves the
 * transcript its share of the row.
 */
export function clampPanelWidth(stored: number | null, rowWidth: number): number {
  const wanted = stored !== null && Number.isFinite(stored) ? stored : PANEL_DEFAULT_WIDTH;
  return Math.max(PANEL_MIN_WIDTH, Math.min(wanted, maxPanelWidth(rowWidth)));
}

export function readStoredPanelWidth(): number | null {
  try {
    const raw = window.localStorage.getItem(PANEL_WIDTH_STORAGE_KEY);
    if (raw === null) return null;
    const n = Number(raw);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

export function writeStoredPanelWidth(width: number): void {
  try {
    window.localStorage.setItem(PANEL_WIDTH_STORAGE_KEY, String(Math.round(width)));
  } catch {
    // Storage blocked: the width just does not persist.
  }
}
