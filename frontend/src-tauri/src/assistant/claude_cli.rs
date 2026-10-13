// assistant/claude_cli.rs
//
// One `claude -p` turn, spawned as its own process, parsed as it streams.
// Port of v1's ClaudeCLI.swift onto tokio::process::Command.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;

/// Tool policy shared with the answer lanes (Task 7). The fast lane never
/// fetches or reads anything; the deep lane reads local files and never
/// writes or reaches the network.
pub const NO_TOOLS: &[&str] = &[
    "Bash", "Edit", "Write", "NotebookEdit", "WebFetch", "WebSearch", "Task", "Read", "Grep",
    "Glob", "TodoWrite",
];
pub const READ_ONLY_TOOLS: &[&str] = &["Read", "Grep", "Glob"];
pub const WRITE_TOOLS: &[&str] = &[
    "Bash", "Edit", "Write", "NotebookEdit", "WebFetch", "WebSearch", "Task",
];

#[derive(Debug, Clone, Default)]
pub struct ClaudeTurn {
    pub prompt: String,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub resume: Option<String>,
    pub fork: bool,
    /// Mints a known session id, for the trunk seed.
    pub session_id: Option<String>,
    pub append_system_prompt: Option<String>,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub partial: bool,
    /// Drops CLAUDE.md discovery, skills, plugins, hooks, MCP servers and the
    /// account's output style, while leaving auth, model choice and the built-in
    /// tools alone. Every lane turn sets this; it cuts the cached prefix and
    /// protects the card format from being rewritten by an output style.
    pub safe_mode: bool,
    /// Extra directories the turn may read.
    pub add_dirs: Vec<PathBuf>,
}

impl ClaudeTurn {
    pub fn argv(&self) -> Vec<String> {
        let mut a = vec![
            "-p".to_string(),
            self.prompt.clone(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
        ];
        // Every lane forks one trunk and passes its own --append-system-prompt.
        // With the CLI default (on), a resumed or forked session keeps the
        // trunk's recorded prompt and ignores the new one, so each turn turns
        // the record off. Source: `claude --help`, --system-prompt-snapshot,
        // CLI 2.1.296.
        a.push("--system-prompt-snapshot".to_string());
        a.push("off".to_string());
        if self.safe_mode {
            a.push("--safe-mode".to_string());
        }
        if !self.add_dirs.is_empty() {
            a.push("--add-dir".to_string());
            for d in &self.add_dirs {
                a.push(d.to_string_lossy().to_string());
            }
        }
        if let Some(resume) = &self.resume {
            a.push("--resume".to_string());
            a.push(resume.clone());
        }
        if self.fork {
            a.push("--fork-session".to_string());
        }
        if let Some(session_id) = &self.session_id {
            a.push("--session-id".to_string());
            a.push(session_id.clone());
        }
        if let Some(model) = &self.model {
            a.push("--model".to_string());
            a.push(model.clone());
        }
        if let Some(effort) = &self.effort {
            a.push("--effort".to_string());
            a.push(effort.clone());
        }
        if self.partial {
            a.push("--include-partial-messages".to_string());
        }
        if let Some(sp) = &self.append_system_prompt {
            a.push("--append-system-prompt".to_string());
            a.push(sp.clone());
        }
        if !self.allowed_tools.is_empty() {
            a.push("--allowedTools".to_string());
            a.push(self.allowed_tools.join(" "));
        }
        if !self.disallowed_tools.is_empty() {
            a.push("--disallowedTools".to_string());
            a.push(self.disallowed_tools.join(" "));
        }
        a
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClaudeEvent {
    SessionId(String),
    TextDelta(String),
    Final(String),
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeOutcome {
    pub session_id: Option<String>,
    pub text: String,
    pub error: Option<String>,
}

/// Parses one NDJSON line from `claude --output-format stream-json`. On a
/// result line reporting an error, the error takes priority over the result
/// text for this line; `run_turn` still keeps whatever text streamed in
/// before the error, matching v1's fall back to the streamed text.
pub fn parse_line(line: &str) -> Option<ClaudeEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let ty = v.get("type")?.as_str()?;
    match ty {
        "system" => {
            if v.get("subtype").and_then(|s| s.as_str()) == Some("init") {
                let sid = v.get("session_id")?.as_str()?.to_string();
                Some(ClaudeEvent::SessionId(sid))
            } else {
                None
            }
        }
        "result" => {
            let is_error = v
                .get("is_error")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            let text = v
                .get("result")
                .and_then(|r| r.as_str())
                .map(|s| s.to_string());
            if is_error {
                Some(ClaudeEvent::Failed(
                    text.unwrap_or_else(|| "claude reported an error".to_string()),
                ))
            } else {
                text.map(ClaudeEvent::Final)
            }
        }
        "stream_event" => {
            let ev = v.get("event")?;
            if ev.get("type").and_then(|s| s.as_str()) != Some("content_block_delta") {
                return None;
            }
            let delta = ev.get("delta")?;
            if delta.get("type").and_then(|s| s.as_str()) != Some("text_delta") {
                return None;
            }
            let text = delta.get("text")?.as_str()?.to_string();
            Some(ClaudeEvent::TextDelta(text))
        }
        _ => None,
    }
}

/// A running turn. Hold it to stop the turn early via `kill()`.
pub struct TurnHandle {
    child: Arc<AsyncMutex<Child>>,
}

impl TurnHandle {
    pub async fn kill(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}

/// Keeps only the last `max_len` chars of a rolling text buffer.
fn tail_chars(s: &mut String, max_len: usize) {
    if s.chars().count() > max_len {
        let start = s.chars().count() - max_len;
        *s = s.chars().skip(start).collect();
    }
}

fn last_chars(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len <= n {
        s.to_string()
    } else {
        s.chars().skip(len - n).collect()
    }
}

/// Spawn one `claude -p` turn; deltas stream through `on_delta`, and the
/// in-flight child is handed to `register` so callers can abort early via
/// the returned `TurnHandle`.
pub async fn run_turn(
    binary: &Path,
    turn: ClaudeTurn,
    mut on_delta: impl FnMut(&str) + Send,
    register: impl FnOnce(TurnHandle),
) -> ClaudeOutcome {
    let mut cmd = Command::new(binary);
    cmd.args(turn.argv())
        .current_dir(&turn.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ClaudeOutcome {
                session_id: None,
                text: String::new(),
                error: Some(format!("could not start claude: {}", e)),
            };
        }
    };

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");

    let child = Arc::new(AsyncMutex::new(child));
    register(TurnHandle {
        child: child.clone(),
    });

    // Drain stderr concurrently so a full pipe never stalls the child while
    // we're blocked reading stdout.
    let stderr_tail = Arc::new(std::sync::Mutex::new(String::new()));
    let stderr_task = {
        let tail = stderr_tail.clone();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut t = tail.lock().unwrap();
                t.push_str(&line);
                t.push('\n');
                tail_chars(&mut t, 1500);
            }
        })
    };

    let mut session_id = None;
    let mut final_text = String::new();
    let mut streamed = String::new();
    let mut error = None;

    let mut stdout_lines = tokio::io::BufReader::new(stdout).lines();
    while let Ok(Some(line)) = stdout_lines.next_line().await {
        match parse_line(&line) {
            Some(ClaudeEvent::SessionId(s)) => session_id = Some(s),
            Some(ClaudeEvent::TextDelta(t)) => {
                streamed.push_str(&t);
                on_delta(&t);
            }
            Some(ClaudeEvent::Final(t)) => final_text = t,
            Some(ClaudeEvent::Failed(e)) => error = Some(e),
            None => {}
        }
    }

    let status = child.lock().await.wait().await;
    let _ = stderr_task.await;

    if let Ok(status) = status {
        if !status.success() {
            let tail = stderr_tail.lock().unwrap().clone();
            error = Some(format!(
                "claude exited {}: {}",
                status.code().unwrap_or(-1),
                last_chars(&tail, 400)
            ));
        }
    }

    ClaudeOutcome {
        session_id,
        text: if final_text.is_empty() {
            streamed
        } else {
            final_text
        },
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_matches_v1_shape() {
        let t = ClaudeTurn {
            prompt: "hi".into(),
            safe_mode: true,
            partial: true,
            resume: Some("abc".into()),
            fork: true,
            model: Some("claude-sonnet-5".into()),
            effort: Some("low".into()),
            ..Default::default()
        };
        let a = t.argv();
        assert_eq!(
            &a[..5],
            &["-p", "hi", "--output-format", "stream-json", "--verbose"]
        );
        assert!(a.windows(2).any(|w| w == ["--resume", "abc"]));
        assert!(a.contains(&"--fork-session".to_string()));
    }

    #[test]
    fn argv_orders_flags_like_v1() {
        let t = ClaudeTurn {
            prompt: "hi".into(),
            safe_mode: true,
            add_dirs: vec![PathBuf::from("/tmp/a")],
            resume: Some("r1".into()),
            fork: true,
            session_id: Some("s1".into()),
            model: Some("m".into()),
            effort: Some("e".into()),
            partial: true,
            append_system_prompt: Some("sp".into()),
            allowed_tools: vec!["Read".into(), "Grep".into()],
            disallowed_tools: vec!["Bash".into()],
            ..Default::default()
        };
        let a = t.argv();
        let expected = vec![
            "-p",
            "hi",
            "--output-format",
            "stream-json",
            "--verbose",
            "--system-prompt-snapshot",
            "off",
            "--safe-mode",
            "--add-dir",
            "/tmp/a",
            "--resume",
            "r1",
            "--fork-session",
            "--session-id",
            "s1",
            "--model",
            "m",
            "--effort",
            "e",
            "--include-partial-messages",
            "--append-system-prompt",
            "sp",
            "--allowedTools",
            "Read Grep",
            "--disallowedTools",
            "Bash",
        ];
        assert_eq!(a, expected);
    }

    #[test]
    fn argv_always_turns_the_system_prompt_snapshot_off() {
        let a = ClaudeTurn::default().argv();
        assert!(a
            .windows(2)
            .any(|w| w == ["--system-prompt-snapshot", "off"]));
    }

    #[test]
    fn parses_stream_events() {
        let init = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;
        let delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hel"}}}"#;
        let result = r#"{"type":"result","result":"Hello","is_error":false}"#;
        assert!(matches!(parse_line(init), Some(ClaudeEvent::SessionId(s)) if s == "s1"));
        assert!(matches!(parse_line(delta), Some(ClaudeEvent::TextDelta(t)) if t == "Hel"));
        assert!(matches!(parse_line(result), Some(ClaudeEvent::Final(t)) if t == "Hello"));
    }

    #[test]
    fn parses_error_result() {
        let err = r#"{"type":"result","result":"boom","is_error":true}"#;
        assert!(matches!(parse_line(err), Some(ClaudeEvent::Failed(t)) if t == "boom"));
    }

    #[test]
    fn ignores_unknown_and_blank_lines() {
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("   "), None);
        assert_eq!(parse_line(r#"{"type":"assistant"}"#), None);
        assert_eq!(parse_line("not json"), None);
    }

    /// Runs a real `claude -p` turn on subscription auth. Gated on the
    /// ASSISTANT_CLAUDE_TEST env var in addition to #[ignore], so a plain
    /// `cargo test -- --ignored` never spends a real turn by accident.
    #[tokio::test]
    #[ignore]
    async fn live_one_turn() {
        if std::env::var("ASSISTANT_CLAUDE_TEST").ok().as_deref() != Some("1") {
            eprintln!("skipping live_one_turn: set ASSISTANT_CLAUDE_TEST=1 to run");
            return;
        }
        let bin = crate::assistant::settings::resolve_claude_binary(
            &crate::assistant::AssistantSettings::default(),
        )
        .expect("claude binary not found");
        let out = run_turn(
            &bin,
            ClaudeTurn {
                prompt: "Reply with exactly OK".into(),
                model: Some("claude-haiku-4-5-20251001".into()),
                partial: false,
                safe_mode: true,
                disallowed_tools: NO_TOOLS.iter().map(|s| s.to_string()).collect(),
                cwd: std::env::temp_dir(),
                ..Default::default()
            },
            |_| {},
            |_| {},
        )
        .await;
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(out.text.contains("OK"));
    }
}
