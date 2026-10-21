// assistant/todo_list.rs
//
// The live To do list: what Joseph promised on the call (You) and what the
// other side owes him (Them). The ask lane's replies carry TODO lines next to
// its questions; this module holds the shapes the panel and the meeting's
// saved history share. Pure data and functions, no I/O.

use serde::Serialize;

/// Who has to do it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TodoOwner {
    /// Joseph promised it, or agreed when the other side asked him.
    You,
    /// Someone on the other side promised it to Joseph.
    Them,
}

/// Where a to-do came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TodoOrigin {
    /// Picked up from the transcript by the assistant.
    Call,
    /// Joseph typed it in the Ask box after `todo:`.
    Typed,
}

/// The call showed the to-do is finished.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoDone {
    /// A few words on how the call showed it, e.g. "said it was sent".
    pub how: String,
    /// Seconds from recording start (the transcript's audio_start_time scale).
    pub call_time: f64,
    /// Epoch milliseconds.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub id: String,
    /// The to-do in a few plain words, e.g. "Send the design doc".
    pub text: String,
    pub owner: TodoOwner,
    /// For a Them to-do, the person's name when the call says it; else "".
    pub who: String,
    /// When it is due, as the call said it ("By Friday", "Monday"); else "".
    pub due: String,
    pub origin: TodoOrigin,
    /// Seconds from recording start (the transcript's audio_start_time
    /// scale): when it was said, or typed.
    pub call_time: f64,
    /// Epoch milliseconds it was added.
    pub at: i64,
    /// Set once the call shows it is finished. Joseph never ticks it off.
    pub done: Option<TodoDone>,
}
