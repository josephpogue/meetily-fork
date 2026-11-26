// The note bar's Title box rule.
// Design: docs/superpowers/specs/2026-11-25-meeting-title-design.md

/** Sent by the app with a drafted note (Rust `meeting_title::TitleSuggestions`). */
export type TitleSuggestions = {
  /** The framework title for each listed project slug. */
  byProject: Record<string, string>;
  /** The title when No project is picked. */
  noProject: string;
  /** The meeting's name when the note was drafted. */
  current: string;
  /** True when the current name stays; every value above is then `current`. */
  keepCurrent: boolean;
};

/**
 * What the Title box shows for the picked project (`choice`, '' for No
 * project): Joseph's own text once he has edited the box (`edited` is that
 * text), else the suggestion for the pick.
 */
export function titleBoxText(s: TitleSuggestions, choice: string, edited: string | null): string {
  if (edited !== null) return edited;
  if (s.keepCurrent) return s.current;
  return s.byProject[choice] ?? s.noProject;
}
