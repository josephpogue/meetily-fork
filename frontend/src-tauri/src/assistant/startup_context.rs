// assistant/startup_context.rs
//
// One small fixed startup context for every Claude call the assistant makes,
// the way a Claude Code session loads CLAUDE.md: the vault page index, the
// routing registry, Joseph's profile, plus the vault page of the project or
// client the meeting brief names. `with_context` wraps a `Runner` and appends
// the context to each turn's `--append-system-prompt`, reading a shared handle
// at call time so a brief change reaches the next turn of every lane.
// `--safe-mode` stays on for every turn, so nothing else loads.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use super::lanes::Runner;

/// Shared between `AssistantCore` and the wrapped runner.
pub type ContextHandle = Arc<RwLock<Arc<str>>>;

pub fn new_handle() -> ContextHandle {
    Arc::new(RwLock::new(Arc::from("")))
}

pub fn set_context(handle: &ContextHandle, text: &str) {
    let mut w = handle.write().unwrap_or_else(|p| p.into_inner());
    *w = Arc::from(text);
}

fn current(handle: &ContextHandle) -> Arc<str> {
    handle.read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Wraps `inner` so each turn's system prompt is `"{lane prompt}\n\n{context}"`.
/// An empty context leaves the turn untouched.
pub fn with_context(inner: Runner, handle: ContextHandle) -> Runner {
    Arc::new(move |mut turn, on_delta, register| {
        let ctx = current(&handle);
        if !ctx.is_empty() {
            turn.append_system_prompt = Some(match turn.append_system_prompt.take() {
                Some(p) if !p.is_empty() => format!("{p}\n\n{ctx}"),
                _ => ctx.to_string(),
            });
        }
        inner(turn, on_delta, register)
    })
}

const PAGE_CAP: usize = 12_000;
const PAGE_CUT_FLOOR: usize = 6_000;

const HEADER: &str = "# Startup context\n\nYou are running inside Joseph's meeting assistant during a live call. Read only: never write, edit or create anything, never run commands, and never start helpers, subagents or agents. Answer briefly, in the shape your instructions above ask for. The sections below are reference for what Joseph knows.";

/// The three fixed files, read once at session open, plus the page index text
/// kept for re-resolving the project page when the brief changes.
#[derive(Debug, Clone)]
pub struct Startup {
    home: PathBuf,
    index: String,
    fixed: String,
}

fn read_or_warn(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("assistant: startup context skipped {}: {}", path.display(), e);
            None
        }
    }
}

/// `/Users/joseph.pogue` becomes `-Users-joseph-pogue`.
fn project_dir_name(home: &Path) -> String {
    home.to_string_lossy().replace('/', "-")
}

fn index_path(home: &Path) -> PathBuf {
    home.join("brain/wiki/Wiki/index.md")
}

fn page_path(home: &Path, slug: &str) -> PathBuf {
    home.join("brain/wiki/Wiki/Projects").join(format!("{slug}.md"))
}

impl Startup {
    /// Reads the fixed files from the real home folder.
    pub fn load_from_home() -> Option<Self> {
        match dirs::home_dir() {
            Some(h) => Some(Self::load(&h)),
            None => {
                log::warn!("assistant: startup context skipped, no home folder");
                None
            }
        }
    }

    pub fn load(home: &Path) -> Self {
        let idx_path = index_path(home);
        let index = read_or_warn(&idx_path);
        let mut fixed = String::from(HEADER);
        let mut section = |title: &str, path: &Path, text: &Option<String>| {
            if let Some(t) = text {
                fixed.push_str(&format!("\n\n## {title} ({})\n{}", path.display(), t.trim_end()));
            }
        };
        section("Vault page index", &idx_path, &index);
        let reg_path = home.join(".claude/routing/registry.md");
        section("Where each department and repo lives", &reg_path, &read_or_warn(&reg_path));
        let prof_path = home
            .join(".claude/projects")
            .join(project_dir_name(home))
            .join("memory/user_profile.md");
        section("Who Joseph is", &prof_path, &read_or_warn(&prof_path));
        Startup { home: home.to_path_buf(), index: index.unwrap_or_default(), fixed }
    }

    /// The full context for `brief`, and the project slug it matched.
    pub fn render(&self, brief: &str) -> (String, Option<String>) {
        let slug = resolve_project(&self.index, brief);
        let mut out = self.fixed.clone();
        if let Some(slug) = &slug {
            let path = page_path(&self.home, slug);
            if let Some(page) = read_or_warn(&path) {
                out.push_str(&format!(
                    "\n\n## Project page named in the brief: {slug} ({})\n{}",
                    path.display(),
                    cap_page(&page, &path.to_string_lossy()).trim_end()
                ));
            }
        }
        (out, slug)
    }
}

/// Logs the one info line the open and each brief re-resolve owe.
pub fn log_built(context: &str, slug: Option<&str>, why: &str) {
    log::info!(
        "assistant: startup context {} chars={} project={}",
        why,
        context.chars().count(),
        slug.unwrap_or("none")
    );
}

/// `[[Projects/<slug>]]` and `[[Projects/<slug>|alias]]` links, in index order.
fn candidates(index: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = index;
    while let Some(i) = rest.find("[[Projects/") {
        rest = &rest[i + "[[Projects/".len()..];
        let end = rest.find("]]").unwrap_or(rest.len());
        let inner = &rest[..end];
        let slug = inner.split('|').next().unwrap_or("").trim();
        // `[[Projects/foo#Section]]` and `[[Projects/foo.md]]` both mean `foo`.
        let slug = slug.split('#').next().unwrap_or("").trim();
        let slug = slug.strip_suffix(".md").unwrap_or(slug);
        let safe = !slug.is_empty()
            && !slug.contains(['/', '\\', '[', ']', '\n'])
            && !slug.starts_with('.');
        if safe && !out.iter().any(|s| s == slug) {
            out.push(slug.to_string());
        }
    }
    out
}

/// Lowercase, every non-alphanumeric char a space, spaces collapsed.
fn normalise(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_lowercase().collect::<String>() } else { " ".to_string() })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The project page slug the brief names, if any. See the module tests for
/// the expected values.
pub fn resolve_project(index: &str, brief: &str) -> Option<String> {
    let cands = candidates(index);
    let hay = format!(" {} ", normalise(brief));
    if hay.trim().is_empty() {
        return None;
    }
    let mut best: Option<(usize, &String)> = None;
    for slug in &cands {
        let mut phrases = vec![normalise(slug)];
        if let Some((other, rest)) = slug.split_once('-') {
            if cands.iter().any(|c| c == other) {
                phrases.push(normalise(rest));
            }
        }
        for p in phrases {
            if p.is_empty() || !hay.contains(&format!(" {p} ")) {
                continue;
            }
            let words = p.split(' ').count();
            if best.map_or(true, |(w, _)| words > w) {
                best = Some((words, slug));
            }
        }
    }
    best.map(|(_, s)| s.clone())
}

/// Page text for inlining: whole at or under 12,000 chars, otherwise cut at
/// the last `## ` heading between 6,000 and 12,000 chars, else the last blank
/// line before 12,000, else the last char boundary, plus a pointer line.
pub fn cap_page(page: &str, abs_path: &str) -> String {
    if page.chars().count() <= PAGE_CAP {
        return page.to_string();
    }
    let limit = page.char_indices().nth(PAGE_CAP).map(|(b, _)| b).unwrap_or(page.len());
    let floor = page.char_indices().nth(PAGE_CUT_FLOOR).map(|(b, _)| b).unwrap_or(limit);
    let mut cut: Option<usize> = None;
    let mut line_start = 0usize;
    for line in page.split_inclusive('\n') {
        if line_start > limit {
            break;
        }
        if line_start >= floor && line.starts_with("## ") {
            cut = Some(line_start);
        }
        line_start += line.len();
    }
    let cut = cut
        .or_else(|| page[..limit].rfind("\n\n"))
        .unwrap_or(limit);
    format!(
        "{}\n\n(Page cut here. The full page is {}.)",
        page[..cut].trim_end_matches('\n'),
        abs_path
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::claude_cli::{ClaudeOutcome, ClaudeTurn, NO_TOOLS};
    use std::sync::Mutex;

    const INDEX: &str = "# Wiki\n- [[Projects/fesco]] FESCO\n- [[Projects/mission-control|Mission Control]]\n- [[Projects/broadcom]]\n- [[Projects/broadcom-principal-financial-group]]\n- [[Projects/broadcom-bank-of-new-york-mellon]]\n- [[Projects/broadcom-cleveland-clinic]]\n- [[Projects/maria]]\n";

    #[test]
    fn resolves_fixture_briefs() {
        let cases: &[(&str, Option<&str>)] = &[
            ("Discovery with Cleveland Clinic on the HCX move", Some("broadcom-cleveland-clinic")),
            ("Broadcom Cleveland Clinic kickoff", Some("broadcom-cleveland-clinic")),
            ("BNY call: Bank of New York Mellon LUN automation", Some("broadcom-bank-of-new-york-mellon")),
            ("FESCO AI revamp", Some("fesco")),
            ("Mission-Control review", Some("mission-control")),
            ("Broadcom PSO onboarding", Some("broadcom")),
            ("fescogroup site review", None),
            ("weekly sync", None),
            ("", None),
        ];
        for (brief, want) in cases {
            assert_eq!(resolve_project(INDEX, brief).as_deref(), *want, "brief {brief:?}");
        }
    }

    #[test]
    fn candidates_in_index_order_with_aliases() {
        assert_eq!(
            candidates(INDEX),
            vec![
                "fesco", "mission-control", "broadcom", "broadcom-principal-financial-group",
                "broadcom-bank-of-new-york-mellon", "broadcom-cleveland-clinic", "maria"
            ]
        );
    }

    #[test]
    fn tie_goes_to_the_earlier_candidate() {
        let idx = "[[Projects/alpha-beta]] [[Projects/beta-alpha]]";
        assert_eq!(resolve_project(idx, "alpha beta and beta alpha").as_deref(), Some("alpha-beta"));
    }

    #[test]
    fn heading_and_extension_links_resolve_to_the_slug() {
        let idx = "[[Projects/foo#Section]] [[Projects/bar.md]] [[Projects/baz.md#Top|Baz]] [[Projects/foo]]";
        assert_eq!(candidates(idx), vec!["foo", "bar", "baz"]);
        assert_eq!(resolve_project(idx, "bar sync").as_deref(), Some("bar"));
    }

    #[test]
    fn unsafe_slugs_are_ignored() {
        assert!(candidates("[[Projects/../secret]] [[Projects/.hidden]]").is_empty());
    }

    #[test]
    fn short_page_goes_in_whole() {
        let p = "x".repeat(12_000);
        assert_eq!(cap_page(&p, "/p.md"), p);
    }

    #[test]
    fn long_page_cuts_at_last_heading_in_window() {
        let mut page = String::new();
        let mut push_at = |page: &mut String, pos: usize, text: &str| {
            while page.chars().count() < pos - 1 {
                page.push('a');
            }
            page.push('\n');
            page.push_str(text);
        };
        push_at(&mut page, 5_000, "## Early\n");
        push_at(&mut page, 9_000, "## Middle\n");
        push_at(&mut page, 14_000, "## Late\n");
        while page.chars().count() < 30_000 {
            page.push('b');
        }
        let heading_at = |p: &str, h: &str| p[..p.find(h).unwrap()].chars().count();
        assert_eq!(heading_at(&page, "## Early"), 5_000);
        assert_eq!(heading_at(&page, "## Middle"), 9_000);
        let out = cap_page(&page, "/abs/p.md");
        assert!(out.ends_with("\n\n(Page cut here. The full page is /abs/p.md.)"));
        assert!(!out.contains("## Middle"));
        assert!(out.contains("## Early"));
        let kept = out.strip_suffix("\n\n(Page cut here. The full page is /abs/p.md.)").unwrap();
        assert_eq!(kept.chars().count(), 8_999, "cut just before the heading at 9,000");
    }

    #[test]
    fn long_page_without_headings_cuts_at_blank_line_then_boundary() {
        let page = format!("{}\n\n{}", "a".repeat(10_000), "b".repeat(5_000));
        let out = cap_page(&page, "/p.md");
        assert!(out.starts_with(&"a".repeat(10_000)));
        assert!(!out.contains('b'));

        let multibyte = "é".repeat(20_000);
        let out = cap_page(&multibyte, "/p.md");
        let kept = out.strip_suffix("\n\n(Page cut here. The full page is /p.md.)").unwrap();
        assert_eq!(kept.chars().count(), 12_000);
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fake_home() -> PathBuf {
        let home = std::env::temp_dir().join(format!("startup-ctx-{}", uuid::Uuid::new_v4()));
        write(&index_path(&home), INDEX);
        write(&home.join(".claude/routing/registry.md"), "REGISTRY");
        write(
            &home.join(".claude/projects").join(project_dir_name(&home)).join("memory/user_profile.md"),
            "PROFILE",
        );
        write(&page_path(&home, "fesco"), "FESCO PAGE");
        home
    }

    #[test]
    fn builds_sections_in_order_and_drops_missing_ones() {
        let home = fake_home();
        let s = Startup::load(&home);
        let (ctx, slug) = s.render("FESCO AI revamp");
        assert_eq!(slug.as_deref(), Some("fesco"));
        assert!(ctx.starts_with("# Startup context\n\nYou are running inside"));
        let order: Vec<usize> = [
            "## Vault page index (",
            "## Where each department and repo lives (",
            "## Who Joseph is (",
            "## Project page named in the brief: fesco (",
            "FESCO PAGE",
        ]
        .iter()
        .map(|h| ctx.find(h).unwrap_or_else(|| panic!("missing {h}")))
        .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]));

        let (ctx, slug) = s.render("weekly sync");
        assert_eq!(slug, None);
        assert!(!ctx.contains("## Project page"));

        std::fs::remove_file(home.join(".claude/routing/registry.md")).unwrap();
        let (ctx, _) = Startup::load(&home).render("");
        assert!(!ctx.contains("## Where each department"));
        assert!(ctx.contains("## Who Joseph is ("));
        let _ = std::fs::remove_dir_all(&home);
    }

    fn recording_runner(seen: Arc<Mutex<Vec<ClaudeTurn>>>) -> Runner {
        Arc::new(move |turn, _d, _r| {
            seen.lock().unwrap().push(turn);
            Box::pin(async { ClaudeOutcome::default() })
        })
    }

    #[tokio::test]
    async fn wrapper_appends_after_the_lane_prompt_and_reads_at_call_time() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handle = new_handle();
        let run = with_context(recording_runner(seen.clone()), handle.clone());
        let turn = |sp: Option<&str>| ClaudeTurn {
            append_system_prompt: sp.map(str::to_string),
            ..Default::default()
        };
        run(turn(Some("LANE")), Box::new(|_| {}), Box::new(|_| {})).await;
        set_context(&handle, "CTX1");
        run(turn(Some("LANE")), Box::new(|_| {}), Box::new(|_| {})).await;
        set_context(&handle, "CTX2");
        run(turn(None), Box::new(|_| {}), Box::new(|_| {})).await;
        let calls = seen.lock().unwrap();
        assert_eq!(calls[0].append_system_prompt.as_deref(), Some("LANE"));
        assert_eq!(calls[1].append_system_prompt.as_deref(), Some("LANE\n\nCTX1"));
        assert_eq!(calls[2].append_system_prompt.as_deref(), Some("CTX2"));
    }

    // ---- live checks, real claude, gated on ASSISTANT_CLAUDE_TEST=1 ----

    fn live_enabled(name: &str) -> bool {
        if std::env::var("ASSISTANT_CLAUDE_TEST").ok().as_deref() != Some("1") {
            eprintln!("skipping {name}: set ASSISTANT_CLAUDE_TEST=1 to run");
            return false;
        }
        true
    }

    fn live_runner() -> Runner {
        let bin = crate::assistant::settings::resolve_claude_binary(
            &crate::assistant::AssistantSettings::default(),
        )
        .expect("claude binary not found");
        crate::assistant::lanes::claude_runner(bin)
    }

    async fn run_one(run: &Runner, turn: ClaudeTurn) -> ClaudeOutcome {
        let out = run(turn, Box::new(|_| {}), Box::new(|_| {})).await;
        assert!(out.error.is_none(), "{:?}", out.error);
        out
    }

    /// Check 1: a resumed and a forked turn carry the context current at
    /// that turn, not the one the trunk was seeded with.
    #[tokio::test]
    #[ignore]
    async fn live_resume_and_fork_carry_the_current_context() {
        if !live_enabled("live_resume_and_fork_carry_the_current_context") {
            return;
        }
        let handle = new_handle();
        let run = with_context(live_runner(), handle.clone());
        let ask = "Reply with only the codeword written in your system prompt.";
        let base = |prompt: &str| ClaudeTurn {
            prompt: prompt.into(),
            cwd: std::env::temp_dir(),
            model: Some("claude-sonnet-5".into()),
            safe_mode: true,
            disallowed_tools: NO_TOOLS.iter().map(|s| s.to_string()).collect(),
            append_system_prompt: Some("LANE PROMPT".into()),
            ..Default::default()
        };
        set_context(&handle, "The codeword is ALPHA.");
        let sid = uuid::Uuid::new_v4().to_string();
        let seed = run_one(&run, ClaudeTurn { session_id: Some(sid.clone()), ..base(ask) }).await;
        println!("seed reply: {}", seed.text);
        set_context(&handle, "The codeword is BRAVO.");
        let resumed = run_one(&run, ClaudeTurn { resume: Some(sid.clone()), ..base(ask) }).await;
        println!("resume reply: {}", resumed.text);
        let forked = run_one(&run, ClaudeTurn { resume: Some(sid), fork: true, ..base(ask) }).await;
        println!("resume+fork reply: {}", forked.text);
        assert!(seed.text.contains("ALPHA"), "{}", seed.text);
        assert!(resumed.text.contains("BRAVO") && !resumed.text.contains("ALPHA"), "{}", resumed.text);
        assert!(forked.text.contains("BRAVO") && !forked.text.contains("ALPHA"), "{}", forked.text);
    }

    fn fast_turn(prompt: &str) -> ClaudeTurn {
        ClaudeTurn {
            prompt: prompt.into(),
            cwd: std::env::temp_dir(),
            model: Some("claude-sonnet-5".into()),
            effort: Some("medium".into()),
            safe_mode: true,
            disallowed_tools: NO_TOOLS.iter().map(|s| s.to_string()).collect(),
            append_system_prompt: Some(include_str!("prompts/fast.md").to_string()),
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            ..Default::default()
        }
    }

    /// Check 2: the real home files and project page reach a fast-lane-shaped turn.
    #[tokio::test]
    #[ignore]
    async fn live_project_page_reaches_the_fast_lane() {
        if !live_enabled("live_project_page_reaches_the_fast_lane") {
            return;
        }
        let startup = Startup::load_from_home().expect("home");
        let (ctx, slug) = startup.render("FESCO AI revamp kickoff");
        println!("context chars={} project={:?}", ctx.chars().count(), slug);
        let handle = new_handle();
        set_context(&handle, &ctx);
        let run = with_context(live_runner(), handle);
        let out = run_one(&run, fast_turn("What year was FESCO founded?")).await;
        println!("fast reply: {}", out.text);
        assert!(out.text.contains("1958"), "{}", out.text);
    }

    /// Check 3: full reply time with and without the context, alternating.
    #[tokio::test]
    #[ignore]
    async fn live_timing_with_and_without_context() {
        if !live_enabled("live_timing_with_and_without_context") {
            return;
        }
        let startup = Startup::load_from_home().expect("home");
        let (ctx, _) = startup.render("FESCO AI revamp kickoff");
        let (mut with, mut without) = (Vec::new(), Vec::new());
        for i in 0..6 {
            let on = i % 2 == 0;
            let handle = new_handle();
            set_context(&handle, if on { &ctx } else { "" });
            let run = with_context(live_runner(), handle);
            let t = std::time::Instant::now();
            run_one(&run, fast_turn("What year was FESCO founded?")).await;
            let ms = t.elapsed().as_millis();
            println!("run {} context {}: {} ms", i / 2 + 1, if on { "ON " } else { "OFF" }, ms);
            if on { with.push(ms) } else { without.push(ms) }
        }
        let median = |v: &mut Vec<u128>| {
            v.sort();
            v[v.len() / 2]
        };
        println!("RESULT context ON  median {} ms", median(&mut with));
        println!("RESULT context OFF median {} ms", median(&mut without));
    }
}
