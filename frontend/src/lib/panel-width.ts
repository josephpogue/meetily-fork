/** Width rules for the assistant panel, which Joseph can drag wider or narrower. */

export const PANEL_DEFAULT_WIDTH = 600;
export const PANEL_MIN_WIDTH = 440;
/** The transcript always keeps at least this much of the row. */
export const TRANSCRIPT_MIN_WIDTH = 420;
export const PANEL_WIDTH_STORAGE_KEY = 'meetily-fork.assistant-panel-width';
/** Arrow keys on the drag handle move the edge by this much. */
export const PANEL_KEY_STEP = 16;

/** Below this row width the transcript can no longer keep its share beside a minimum-width panel. */
const NARROW_ROW = TRANSCRIPT_MIN_WIDTH + PANEL_MIN_WIDTH;
/** In a narrow row the panel still shows at least this much, unless the row itself is smaller. */
export const PANEL_NARROW_MIN_WIDTH = 240;

/**
 * The widest the panel may be in a row of this width. A row that leaves the
 * transcript its share beside a minimum panel allows row - 420; a narrower row
 * gives the panel what the transcript's share leaves (at least 240), never
 * more than the row.
 */
export function maxPanelWidth(rowWidth: number): number {
  if (rowWidth >= NARROW_ROW) return rowWidth - TRANSCRIPT_MIN_WIDTH;
  return Math.min(rowWidth, Math.max(PANEL_NARROW_MIN_WIDTH, rowWidth - TRANSCRIPT_MIN_WIDTH));
}

/** The narrowest the panel may be in a row of this width. */
export function minPanelWidth(rowWidth: number): number {
  return rowWidth >= NARROW_ROW ? PANEL_MIN_WIDTH : Math.min(PANEL_NARROW_MIN_WIDTH, rowWidth);
}

/**
 * The width to show: the stored choice (or the default when there is none or
 * it is not a finite number), kept between the minimum and what leaves the
 * transcript its share of the row. The stored choice itself is never changed
 * by a narrow window.
 */
export function clampPanelWidth(stored: number | null, rowWidth: number): number {
  const wanted = stored !== null && Number.isFinite(stored) ? stored : PANEL_DEFAULT_WIDTH;
  return Math.max(minPanelWidth(rowWidth), Math.min(wanted, maxPanelWidth(rowWidth)));
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
