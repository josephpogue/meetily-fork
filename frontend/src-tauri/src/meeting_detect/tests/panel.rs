// Tests for meeting_detect/panel.rs: `cards` driven through the public API of `Rules`, and `place`.
// Expected values come from the state table and worked examples in
// docs/superpowers/plans/2026-11-21-floating-meeting-prompt-brief.md.

use super::*;
use crate::audio::call_apps::CallAppId;
use crate::meeting_detect::rules::{AppSignals, Rules, Tick};

fn in_call(title: Option<&str>) -> AppSignals {
    AppSignals {
        mic: true,
        meeting_window: true,
        title: title.map(|t| t.to_string()),
        ..Default::default()
    }
}

fn started(app: CallAppId, title: Option<&str>) -> Card {
    Card { app, kind: CardKind::Started, title: title.map(|t| t.to_string()) }
}

fn ended(app: CallAppId) -> Card {
    Card { app, kind: CardKind::Ended, title: None }
}

/// Zoom (and optionally Teams) prompted, nothing recording.
fn prompted(zoom: Option<&str>, teams: Option<Option<&str>>) -> Rules {
    let mut rules = Rules::new();
    let tick = Tick {
        now: 2,
        recording: false,
        zoom: if zoom.is_some() { in_call(zoom) } else { AppSignals::default() },
        teams: match teams {
            Some(t) => in_call(t),
            None => AppSignals::default(),
        },
        ..Default::default()
    };
    let _ = rules.tick(&tick);
    rules
}

fn zoom_and_teams_prompted() -> Rules {
    prompted(Some("Weekly sync"), Some(None))
}

/// Non-recording ticks with all-default signals every 2 s until nothing is prompted.
fn run_until_cleared(rules: &mut Rules, mut now: u64, teams: AppSignals) {
    for _ in 0..30 {
        if rules.prompted().is_empty() {
            return;
        }
        now += 2;
        let _ = rules.tick(&Tick { now, recording: false, teams: teams.clone(), ..Default::default() });
    }
}

/// A recording during which Zoom was open, then Zoom gone until the Ended prompt shows.
fn zoom_ended() -> (Rules, u64) {
    let mut rules = Rules::new();
    let _ = rules.tick(&Tick {
        now: 0,
        recording: true,
        recording_app: None,
        zoom: in_call(None),
        ..Default::default()
    });
    let mut now = 0;
    for _ in 0..30 {
        if !rules.ended().is_empty() {
            break;
        }
        now += 2;
        let _ = rules.tick(&Tick { now, recording: true, recording_app: None, ..Default::default() });
    }
    assert!(!rules.ended().is_empty(), "setup: the Ended prompt never showed");
    (rules, now)
}

// ---- cards ----

#[test]
fn idle_shows_no_cards() {
    assert_eq!(cards(&Rules::new()), vec![]);
}

#[test]
fn zoom_prompted_shows_one_started_card_with_its_title() {
    let rules = prompted(Some("Weekly sync"), None);
    assert_eq!(cards(&rules), vec![started(CallAppId::Zoom, Some("Weekly sync"))]);
}

#[test]
fn a_prompt_with_no_known_title_has_none() {
    let rules = prompted(None, Some(None));
    assert_eq!(cards(&rules), vec![started(CallAppId::Teams, None)]);
}

#[test]
fn zoom_and_teams_prompted_show_two_started_cards_zoom_first() {
    let rules = zoom_and_teams_prompted();
    assert_eq!(
        cards(&rules),
        vec![started(CallAppId::Zoom, Some("Weekly sync")), started(CallAppId::Teams, None)]
    );
}

#[test]
fn recording_started_removes_every_started_card() {
    let mut rules = zoom_and_teams_prompted();
    let _ = rules.recording_started(Some(CallAppId::Zoom));
    assert_eq!(cards(&rules), vec![]);
}

#[test]
fn recording_started_with_no_app_removes_every_started_card() {
    let mut rules = zoom_and_teams_prompted();
    let _ = rules.recording_started(None);
    assert_eq!(cards(&rules), vec![]);
}

#[test]
fn a_call_that_ends_before_any_click_drops_that_card_only() {
    let mut rules = zoom_and_teams_prompted();
    run_until_cleared_zoom_only(&mut rules);
    assert_eq!(cards(&rules), vec![started(CallAppId::Teams, None)]);
}

/// Zoom gone, Teams still in its call, until Zoom's prompt is cleared.
fn run_until_cleared_zoom_only(rules: &mut Rules) {
    let mut now = 2;
    for _ in 0..30 {
        if !cards(rules).iter().any(|c| c.app == CallAppId::Zoom) {
            return;
        }
        now += 2;
        let _ = rules.tick(&Tick { now, recording: false, teams: in_call(None), ..Default::default() });
    }
}

#[test]
fn when_every_prompted_call_ends_the_window_has_no_cards() {
    let mut rules = prompted(Some("Weekly sync"), None);
    run_until_cleared(&mut rules, 2, AppSignals::default());
    assert_eq!(cards(&rules), vec![]);
}

#[test]
fn dismiss_removes_that_card_and_keeps_the_other() {
    let mut rules = zoom_and_teams_prompted();
    let _ = rules.dismiss(CallAppId::Zoom, 4);
    assert_eq!(cards(&rules), vec![started(CallAppId::Teams, None)]);
}

#[test]
fn a_dismissed_app_shows_no_card_for_30_minutes() {
    let mut rules = prompted(Some("Weekly sync"), None);
    let _ = rules.dismiss(CallAppId::Zoom, 4);
    // 10 minutes later the same call is still going.
    let _ = rules.tick(&Tick { now: 4 + 600, recording: false, zoom: in_call(Some("Weekly sync")), ..Default::default() });
    assert_eq!(cards(&rules), vec![]);
}

#[test]
fn a_new_call_after_30_minutes_prompts_again() {
    let mut rules = prompted(Some("Weekly sync"), None);
    let _ = rules.dismiss(CallAppId::Zoom, 4);
    // The dismissed call ends (a dismissed call never prompts again), then a new call starts.
    for i in 1..=30 {
        let _ = rules.tick(&Tick { now: 4 + 2 * i, recording: false, ..Default::default() });
    }
    let _ = rules.tick(&Tick { now: 4 + 31 * 60, recording: false, zoom: in_call(Some("Weekly sync")), ..Default::default() });
    assert_eq!(cards(&rules), vec![started(CallAppId::Zoom, Some("Weekly sync"))]);
}

#[test]
fn while_recording_no_started_card_shows_for_a_new_call() {
    let mut rules = Rules::new();
    let _ = rules.tick(&Tick { now: 0, recording: true, recording_app: None, zoom: in_call(Some("A")), ..Default::default() });
    assert!(cards(&rules).iter().all(|c| c.kind != CardKind::Started));
}

#[test]
fn a_watched_call_that_ended_shows_one_ended_card_without_a_title() {
    let (rules, _) = zoom_ended();
    assert_eq!(cards(&rules), vec![ended(CallAppId::Zoom)]);
}

#[test]
fn keep_removes_the_ended_card() {
    let (mut rules, _) = zoom_ended();
    let _ = rules.keep(CallAppId::Zoom);
    assert_eq!(cards(&rules), vec![]);
}

#[test]
fn a_recording_stopped_another_way_removes_the_ended_card() {
    let (mut rules, now) = zoom_ended();
    let _ = rules.tick(&Tick { now: now + 2, recording: false, ..Default::default() });
    assert_eq!(cards(&rules), vec![]);
}

#[test]
fn a_rejoined_call_removes_the_ended_card() {
    let (mut rules, now) = zoom_ended();
    let _ = rules.tick(&Tick { now: now + 2, recording: true, recording_app: None, zoom: in_call(None), ..Default::default() });
    assert_eq!(cards(&rules), vec![]);
}

// ---- place ----

#[test]
fn centers_on_the_main_screen_below_the_menu_bar() {
    // (1512 - 360) / 2 = 576
    let got = place(Rect { x: 0.0, y: 25.0, w: 1512.0, h: 920.0 }, 360.0, 120.0);
    assert_eq!(got, (576.0, 25.0 + GAP_TOP));
}

#[test]
fn centers_on_a_second_screen_to_the_right() {
    // 1512 + (1920 - 360) / 2 = 1512 + 780 = 2292
    let got = place(Rect { x: 1512.0, y: 0.0, w: 1920.0, h: 1055.0 }, 360.0, 120.0);
    assert_eq!(got, (2292.0, GAP_TOP));
}

#[test]
fn a_window_wider_than_the_screen_starts_at_its_left_edge() {
    let got = place(Rect { x: 0.0, y: 25.0, w: 300.0, h: 600.0 }, 360.0, 120.0);
    assert_eq!(got, (0.0, 25.0 + GAP_TOP));
}

#[test]
fn a_wide_window_on_an_offset_screen_starts_at_that_screens_left_edge() {
    let got = place(Rect { x: 1512.0, y: 25.0, w: 300.0, h: 600.0 }, 360.0, 120.0);
    assert_eq!(got, (1512.0, 25.0 + GAP_TOP));
}

#[test]
fn a_window_exactly_as_wide_as_the_screen_starts_at_its_left_edge() {
    let got = place(Rect { x: 100.0, y: 0.0, w: 360.0, h: 600.0 }, 360.0, 120.0);
    assert_eq!(got, (100.0, GAP_TOP));
}

#[test]
fn the_gap_is_positive() {
    assert!(GAP_TOP > 0.0);
}
