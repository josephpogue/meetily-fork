// assistant/transcript.rs
//
// Ordered, labeled transcript log built from TranscriptUpdate, plus the
// utterance assembler: consecutive same-speaker segments merge into one
// utterance until a quiet gap or a speaker flip closes it.

use crate::audio::TranscriptUpdate;

/// Segments this close together (by audio_start_time vs. the open
/// utterance's end) keep merging into the same utterance.
const UTTERANCE_GAP_SECS: f64 = 1.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    You,
    Them,
    Unknown,
}

impl Speaker {
    fn from_source(source: &str) -> Self {
        match source {
            "mic" => Speaker::You,
            "system" => Speaker::Them,
            _ => Speaker::Unknown,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Speaker::You => "You",
            Speaker::Them => "Them",
            Speaker::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssemblerOut {
    /// Partial preview, or a still-open utterance that just grew.
    Running { speaker: Speaker, text: String },
    /// Closed: quiet gap >= 1.2 s or a speaker flip.
    Utterance { speaker: Speaker, text: String },
}

struct OpenUtterance {
    speaker: Speaker,
    /// Text merged from finalized segments only; a partial never lands here.
    committed_text: String,
    /// Latest partial preview for the segment still in flight, replaced
    /// wholesale (never appended) when its final arrives.
    pending_partial: Option<String>,
    end_time: f64,
}

#[derive(Default)]
pub struct TranscriptLog {
    finals: Vec<(Speaker, String, f64)>,
    open: Option<OpenUtterance>,
}

impl TranscriptLog {
    pub fn ingest(&mut self, u: &TranscriptUpdate) -> Vec<AssemblerOut> {
        let speaker = Speaker::from_source(&u.source);
        let mut out = Vec::new();

        let continues_open = match &self.open {
            Some(open) => {
                open.speaker == speaker
                    && (u.audio_start_time - open.end_time) < UTTERANCE_GAP_SECS
            }
            None => false,
        };

        if !continues_open {
            if let Some(closed) = self.close_open() {
                out.push(closed);
            }
            self.open = Some(OpenUtterance {
                speaker,
                committed_text: String::new(),
                pending_partial: None,
                end_time: u.audio_end_time,
            });
        }

        let open = self.open.as_mut().expect("just ensured an open utterance");
        if u.is_partial {
            open.pending_partial = Some(u.text.clone());
        } else {
            if !open.committed_text.is_empty() {
                open.committed_text.push(' ');
            }
            open.committed_text.push_str(&u.text);
            open.pending_partial = None;
        }
        open.end_time = u.audio_end_time;

        out.push(AssemblerOut::Running {
            speaker,
            text: self.preview_text(),
        });
        out
    }

    /// Snapshot of the currently-open utterance's text, but only if it is
    /// already attributed to `You`. Voice-ask calls this at hotkey-press
    /// time to mark where an in-progress utterance stood, so speech spoken
    /// before the press can be excluded from what voice-ask hears.
    pub fn open_you_preview(&self) -> String {
        match &self.open {
            Some(open) if open.speaker == Speaker::You => self.preview_text(),
            _ => String::new(),
        }
    }

    /// The still-open utterance's finalized text, labeled, or empty. Not
    /// part of `delta_since`, which covers closed finals only: an utterance
    /// closes when someone else speaks or after a 1.2 s gap is followed by
    /// more speech, so the last thing said before a silence stays open.
    pub fn open_preview(&self) -> String {
        match &self.open {
            Some(open) if !open.committed_text.is_empty() => {
                format!("{}: {}", open.speaker.label(), open.committed_text)
            }
            _ => String::new(),
        }
    }

    fn preview_text(&self) -> String {
        let Some(open) = &self.open else {
            return String::new();
        };
        match &open.pending_partial {
            Some(partial) if !open.committed_text.is_empty() => {
                format!("{} {}", open.committed_text, partial)
            }
            Some(partial) => partial.clone(),
            None => open.committed_text.clone(),
        }
    }

    /// Commits the open utterance's finalized text to the log, if any, and
    /// returns the Utterance event that reports it as closed. An utterance
    /// that never finalized (partial-only) closes silently.
    fn close_open(&mut self) -> Option<AssemblerOut> {
        let open = self.open.take()?;
        if open.committed_text.is_empty() {
            return None;
        }
        self.finals
            .push((open.speaker, open.committed_text.clone(), open.end_time));
        Some(AssemblerOut::Utterance {
            speaker: open.speaker,
            text: open.committed_text,
        })
    }

    /// New finals since `cursor`, labeled "You:"/"Them:", and the cursor to pass next time.
    pub fn delta_since(&self, cursor: usize) -> (String, usize) {
        let start = cursor.min(self.finals.len());
        let text = self.finals[start..]
            .iter()
            .map(|(speaker, text, _)| format!("{}: {}", speaker.label(), text))
            .collect::<Vec<_>>()
            .join("\n");
        (text, self.finals.len())
    }

    /// Closed finals plus the still-open utterance's committed text (not its
    /// in-flight partial preview, which isn't stable yet). Without this, an
    /// utterance is invisible to `window`/`all` until a >=1.2s gap or a
    /// speaker flip closes it -- which a user pressing Explain or Catch-up
    /// moments after someone finishes speaking will routinely beat, and which
    /// one long unbroken utterance may never do at all.
    fn entries(&self) -> impl Iterator<Item = (Speaker, &str, f64)> {
        self.finals
            .iter()
            .map(|(s, t, e)| (*s, t.as_str(), *e))
            .chain(
                self.open
                    .as_ref()
                    .filter(|o| !o.committed_text.is_empty())
                    .map(|o| (o.speaker, o.committed_text.as_str(), o.end_time)),
            )
    }

    /// Labeled finals from the last `seconds` of audio, optionally filtered to one speaker.
    pub fn window(&self, seconds: f64, speaker: Option<Speaker>) -> String {
        let now = self.entries().map(|(_, _, end)| end).fold(0.0_f64, f64::max);

        self.entries()
            .filter(|(_, _, end)| now - end <= seconds)
            .filter(|(s, _, _)| speaker.map(|want| want == *s).unwrap_or(true))
            .map(|(s, text, _)| format!("{}: {}", s.label(), text))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn all(&self) -> String {
        self.entries()
            .map(|(speaker, text, _)| format!("{}: {}", speaker.label(), text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tu(source: &str, text: &str, start: f64, end: f64, is_partial: bool, seq: u64) -> TranscriptUpdate {
        TranscriptUpdate {
            text: text.to_string(),
            timestamp: "00:00:00".to_string(),
            source: source.to_string(),
            sequence_id: seq,
            chunk_start_time: start,
            is_partial,
            confidence: 1.0,
            audio_start_time: start,
            audio_end_time: end,
            duration: end - start,
        }
    }

    #[test]
    fn merges_consecutive_same_speaker_within_gap() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "Hi", 0.0, 1.0, false, 1));
        let out = log.ingest(&tu("system", "there", 1.5, 2.0, false, 2));
        assert_eq!(
            out,
            vec![AssemblerOut::Running {
                speaker: Speaker::Them,
                text: "Hi there".to_string()
            }]
        );
    }

    #[test]
    fn gap_closes_same_speaker_utterance() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "First", 0.0, 1.0, false, 1));
        let out = log.ingest(&tu("system", "Second", 3.0, 4.0, false, 2));
        assert_eq!(
            out,
            vec![
                AssemblerOut::Utterance {
                    speaker: Speaker::Them,
                    text: "First".to_string()
                },
                AssemblerOut::Running {
                    speaker: Speaker::Them,
                    text: "Second".to_string()
                },
            ]
        );
    }

    #[test]
    fn speaker_flip_closes_open_utterance() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "Hi there", 0.0, 1.0, false, 1));
        let out = log.ingest(&tu("mic", "yes", 1.2, 2.0, false, 2));
        assert_eq!(
            out,
            vec![
                AssemblerOut::Utterance {
                    speaker: Speaker::Them,
                    text: "Hi there".to_string()
                },
                AssemblerOut::Running {
                    speaker: Speaker::You,
                    text: "yes".to_string()
                },
            ]
        );
    }

    #[test]
    fn partials_replaced_not_appended_by_final() {
        let mut log = TranscriptLog::default();
        let r1 = log.ingest(&tu("system", "Hel", 0.0, 0.3, true, 5));
        assert_eq!(
            r1,
            vec![AssemblerOut::Running {
                speaker: Speaker::Them,
                text: "Hel".to_string()
            }]
        );
        let r2 = log.ingest(&tu("system", "Hello wor", 0.0, 0.6, true, 5));
        assert_eq!(
            r2,
            vec![AssemblerOut::Running {
                speaker: Speaker::Them,
                text: "Hello wor".to_string()
            }]
        );
        let r3 = log.ingest(&tu("system", "Hello world", 0.0, 1.0, false, 5));
        assert_eq!(
            r3,
            vec![AssemblerOut::Running {
                speaker: Speaker::Them,
                text: "Hello world".to_string()
            }]
        );
    }

    #[test]
    fn delta_since_returns_new_labeled_finals() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("mic", "hello", 0.0, 1.0, false, 1));
        log.ingest(&tu("system", "hi there", 3.0, 4.0, false, 2));
        let (text, cursor) = log.delta_since(0);
        assert_eq!(text, "You: hello");
        assert_eq!(cursor, 1);

        log.ingest(&tu("mic", "ok", 6.0, 6.5, false, 3));
        let (text2, cursor2) = log.delta_since(cursor);
        assert_eq!(text2, "Them: hi there");
        assert_eq!(cursor2, 2);
    }

    #[test]
    fn window_filters_by_speaker_and_recency() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "old them", 0.0, 1.0, false, 1));
        log.ingest(&tu("mic", "you", 1.2, 1.5, false, 2));
        log.ingest(&tu("system", "recent them", 20.0, 21.0, false, 3));
        log.ingest(&tu("mic", "closer", 22.0, 22.5, false, 4));

        let out = log.window(15.0, Some(Speaker::Them));
        assert_eq!(out, "Them: recent them");
    }

    /// Explain's real usage: press it moments after the other side finishes
    /// talking, well inside the 1.2s gap that would otherwise close the
    /// utterance. Reproduces Explain returning nothing even though the
    /// speaker was tagged correctly.
    #[test]
    fn window_sees_a_still_open_utterance() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "Joseph, what does the assistant do", 0.0, 4.0, false, 1));

        let out = log.window(15.0, Some(Speaker::Them));
        assert_eq!(out, "Them: Joseph, what does the assistant do");
    }

    /// Catch-up's first-use path: one long stretch of speech with no gap
    /// ever reaching 1.2s and no speaker flip never closes, so `finals`
    /// stays empty for the whole stretch. Reproduces catch-up finding zero
    /// transcript despite minutes of real speech.
    #[test]
    fn all_sees_a_still_open_utterance_that_never_closed() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("mic", "one", 0.0, 1.0, false, 1));
        log.ingest(&tu("mic", "two", 1.5, 2.5, false, 2));
        log.ingest(&tu("mic", "three", 3.0, 4.0, false, 3));

        assert_eq!(log.all(), "You: one two three");
    }

    /// The in-flight partial preview is not yet finalized text and must not
    /// leak into a lane prompt.
    #[test]
    fn window_excludes_the_open_utterance_pending_partial() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "committed", 0.0, 1.0, false, 1));
        log.ingest(&tu("system", "not yet final", 1.1, 1.4, true, 2));

        let out = log.window(15.0, Some(Speaker::Them));
        assert_eq!(out, "Them: committed");
    }

    /// Voice-ask reads this at hotkey-press time to mark where an
    /// already-in-progress You utterance stood.
    #[test]
    fn open_you_preview_returns_the_in_progress_you_text() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("mic", "Okay.", 0.0, 1.0, false, 1));

        assert_eq!(log.open_you_preview(), "Okay.");
    }

    /// A still-open utterance from the other side must not be mistaken for
    /// pre-hotkey You speech.
    #[test]
    fn open_you_preview_is_empty_when_the_open_utterance_is_them() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "hello", 0.0, 1.0, false, 1));

        assert_eq!(log.open_you_preview(), "");
    }

    /// No speech at all yet.
    #[test]
    fn open_you_preview_is_empty_with_no_open_utterance() {
        let log = TranscriptLog::default();
        assert_eq!(log.open_you_preview(), "");
    }
    #[test]
    fn open_preview_shows_the_open_utterance_that_delta_since_does_not() {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "our fee is ten percent", 0.0, 1.0, false, 1));
        assert_eq!(log.delta_since(0).0, "");
        assert_eq!(log.open_preview(), "Them: our fee is ten percent");

        log.ingest(&tu("mic", "ok", 2.0, 3.0, false, 2));
        assert_eq!(log.delta_since(0).0, "Them: our fee is ten percent");
        assert_eq!(log.open_preview(), "You: ok");
    }

}
