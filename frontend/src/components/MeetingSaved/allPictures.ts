// Shapes of the picture_cleanup_all command (src-tauri/src/assistant/cleanup.rs,
// `AllPictures` and `MeetingPicture`) and the one rule the All pictures panel owns:
// which pictures it shows. The backend writes the wording and the order.

export type PictureReason = 'loading' | 'greyed' | 'blank' | 'popup' | 'before_share' | 'repeat' | 'by_you';

export interface MeetingPicture {
  /** Its place in the frames folder; the name Remove, Restore and Undo take. */
  file: string;
  /** Where the picture is now: `file` when kept, its removed/ path when removed. */
  shownFile: string;
  time: string;
  bytes: number;
  removed: boolean;
  reason: PictureReason | null;
  caption: string | null;
  repeatOf: string | null;
  /** When a removed picture is deleted for good; 0 when kept. */
  deleteAfterMs: number;
}

export interface AllPictures {
  /** "44 pictures kept, 25 MB" */
  title: string;
  kept: number;
  keptBytes: number;
  removed: number;
  /** Kept and removed together, oldest first. */
  pictures: MeetingPicture[];
}

/**
 * The pictures the panel shows, in the backend's order: only kept ones while
 * "Show removed" is off, every picture while it is on. The full-size viewer
 * steps through this same list.
 */
export function shownPictures(all: AllPictures, showRemoved: boolean): MeetingPicture[] {
  void all;
  void showRemoved;
  throw new Error('not implemented');
}

/** "See all 44 pictures", "See 1 picture", or "See all pictures" when none is kept but some are removed; null when the meeting has no picture at all. */
export function seeAllLabel(kept: number, removed: number): string | null {
  void kept;
  void removed;
  throw new Error('not implemented');
}
