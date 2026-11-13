// meeting_detect/rules.rs
//
// The pure per-app call tracker. Each tick the probe hands it what it saw
// (mic, window, frontmost, recording) and it answers with the events to
// raise. No clocks, no I/O: time is whole seconds the caller supplies.

/// A call app tracked on its own.
pub use crate::audio::call_apps::CallAppId as App;

pub const ALL: [App; 2] = App::ALL;

fn index(app: App) -> usize {
    match app {
        App::Zoom => 0,
        App::Teams => 1,
    }
}

/// A call ends after this many quiet ticks in a row.
pub const QUIET_TICKS_TO_END: u8 = 3;
/// Teams: the window counts when Teams came to the front this soon after the mic turned on.
pub const FRONT_WINDOW_SECS: u64 = 30;
/// A dismissal silences the app this long.
pub const DISMISS_SECS: u64 = 30 * 60;

/// What one tick saw of one app.
#[derive(Debug, Clone, Default)]
pub struct AppSignals {
    /// Some process owned by the app is capturing the mic.
    pub mic: bool,
    /// A window that is a meeting window by its title (Zoom's, Teams').
    pub meeting_window: bool,
    /// Teams only: a titled normal-size window that is not a meeting title.
    pub plain_window: bool,
    /// The app was the frontmost app at this tick.
    pub frontmost: bool,
    /// The meeting window's title, already cleaned; None when generic.
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Tick {
    pub now: u64,
    pub recording: bool,
    /// The one app the recording is tied to (started from a detected call, or Call audio set to
    /// one app); None watches both.
    pub recording_app: Option<App>,
    pub zoom: AppSignals,
    pub teams: AppSignals,
}

impl Tick {
    fn signals(&self, app: App) -> &AppSignals {
        match app {
            App::Zoom => &self.zoom,
            App::Teams => &self.teams,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Prompted { app: App, title: Option<String> },
    Cleared { app: App },
    /// A call seen during this recording ended: ask whether to stop recording.
    Ended { app: App },
    /// The ended prompt is gone: kept, the app's call came back, or the recording stopped.
    EndedCleared { app: App },
}

/// Whether a recording tied to `recording_app` watches `app`.
fn watches(recording_app: Option<App>, app: App) -> bool {
    recording_app.map_or(true, |r| r == app)
}

#[derive(Debug, Default)]
struct Call {
    mic_since: Option<u64>,
    front_ok: bool,
    quiet: u8,
    prompted: bool,
    /// Never prompt this call: recording was on, or it was dismissed.
    seen: bool,
    /// Open at some point during the current recording (and inside its app scope).
    in_recording: bool,
    title: Option<String>,
}

#[derive(Debug, Default)]
struct AppState {
    call: Option<Call>,
    dismissed_until: Option<u64>,
    /// The ended prompt is showing for this app.
    ended: bool,
    /// A call seen during the recording ended on this tick.
    just_ended: bool,
}

#[derive(Debug, Default)]
pub struct Rules {
    apps: [AppState; 2],
}

impl Rules {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this tick needs a window listing: only when some app has the
    /// mic on or has an open call, so an idle Mac costs no listing.
    pub fn needs_windows(&self, zoom_mic: bool, teams_mic: bool) -> bool {
        // Any open call needs its window checked, prompted or not: a seen or dismissed call whose
        // mic drops must not end while its meeting window is still open.
        zoom_mic || teams_mic || self.apps.iter().any(|a| a.call.is_some())
    }

    /// A recording is on and this tick could not be read: still mark every open call seen and clear its
    /// prompt, so no prompt outlives a recording start. Call start and end wait for a good read.
    pub fn recording_started(&mut self, recording_app: Option<App>) -> Vec<Event> {
        let mut events = Vec::new();
        for app in ALL {
            if let Some(call) = self.apps[index(app)].call.as_mut() {
                call.seen = true;
                call.in_recording |= watches(recording_app, app);
                if call.prompted {
                    call.prompted = false;
                    events.push(Event::Cleared { app });
                }
            }
        }
        events
    }

    /// Calls prompted right now, with the title shown for each.
    pub fn prompted(&self) -> Vec<(App, Option<String>)> {
        ALL.iter()
            .filter_map(|&app| {
                let c = self.apps[index(app)].call.as_ref()?;
                c.prompted.then(|| (app, c.title.clone()))
            })
            .collect()
    }

    /// Apps whose ended prompt is showing right now.
    pub fn ended(&self) -> Vec<App> {
        ALL.iter().copied().filter(|&a| self.apps[index(a)].ended).collect()
    }

    /// The user kept recording. Returns EndedCleared when the prompt was showing.
    pub fn keep(&mut self, app: App) -> Option<Event> {
        let st = &mut self.apps[index(app)];
        std::mem::take(&mut st.ended).then_some(Event::EndedCleared { app })
    }

    /// The user dismissed this app's prompt. Returns Cleared when a prompt was showing.
    pub fn dismiss(&mut self, app: App, now: u64) -> Option<Event> {
        let st = &mut self.apps[index(app)];
        st.dismissed_until = Some(now + DISMISS_SECS);
        let call = st.call.as_mut()?;
        call.seen = true;
        if call.prompted {
            call.prompted = false;
            return Some(Event::Cleared { app });
        }
        None
    }

    pub fn tick(&mut self, t: &Tick) -> Vec<Event> {
        let mut events = Vec::new();
        for app in ALL {
            self.tick_app(app, t, &mut events);
        }
        self.tick_ended(t, &mut events);
        events
    }

    /// End-of-call prompts. Decided after every app's tick so "no other call open" sees this tick's ends.
    fn tick_ended(&mut self, t: &Tick, events: &mut Vec<Event>) {
        for app in ALL {
            let st = &mut self.apps[index(app)];
            // The recording stopped, or this app's call came back: the prompt has no meaning now.
            if st.ended && (!t.recording || st.call.is_some()) {
                st.ended = false;
                events.push(Event::EndedCleared { app });
            }
        }
        for app in ALL {
            if !self.apps[index(app)].just_ended || !t.recording {
                continue;
            }
            let other_open = ALL.iter().any(|&o| {
                o != app && watches(t.recording_app, o) && self.apps[index(o)].call.is_some()
            });
            let st = &mut self.apps[index(app)];
            if !other_open && !st.ended {
                st.ended = true;
                events.push(Event::Ended { app });
            }
        }
        for st in &mut self.apps {
            st.just_ended = false;
        }
    }

    fn tick_app(&mut self, app: App, t: &Tick, events: &mut Vec<Event>) {
        let s = t.signals(app);
        let st = &mut self.apps[index(app)];
        let active = s.mic || s.meeting_window;
        if st.call.is_none() {
            if !active {
                return;
            }
            st.call = Some(Call::default());
        }
        let call = st.call.as_mut().expect("call set above");

        if s.mic && call.mic_since.is_none() {
            call.mic_since = Some(t.now);
        }
        if app == App::Teams && s.frontmost {
            if let Some(since) = call.mic_since {
                if t.now <= since + FRONT_WINDOW_SECS {
                    call.front_ok = true;
                }
            }
        }

        // The call ends on quiet ticks: strict evidence only, so a Teams main
        // window that stays open after the call cannot keep it alive.
        if active {
            call.quiet = 0;
        } else {
            call.quiet += 1;
        }
        if t.recording && watches(t.recording_app, app) {
            call.in_recording = true;
        } else if !t.recording {
            call.in_recording = false;
        }
        if call.quiet >= QUIET_TICKS_TO_END {
            if call.prompted {
                events.push(Event::Cleared { app });
            }
            st.just_ended = call.in_recording && t.recording;
            st.call = None;
            return;
        }

        if t.recording {
            call.seen = true;
            if call.prompted {
                call.prompted = false;
                events.push(Event::Cleared { app });
            }
            return;
        }

        let window = s.meeting_window || (app == App::Teams && s.plain_window && call.front_ok);
        if window && s.title.is_some() {
            call.title = s.title.clone();
        }
        let silenced = st.dismissed_until.is_some_and(|until| t.now < until);
        if s.mic && window && !call.prompted && !call.seen && !silenced {
            call.prompted = true;
            events.push(Event::Prompted { app, title: s.title.clone() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One tick of scripted signals; time is the tick's seconds.
    fn sig(mic: bool, win: bool) -> AppSignals {
        AppSignals { mic, meeting_window: win, ..Default::default() }
    }

    fn tick(now: u64, recording: bool, zoom: AppSignals, teams: AppSignals) -> Tick {
        Tick { now, recording, zoom, teams, ..Default::default() }
    }

    fn zoom_only(now: u64, mic: bool, win: bool) -> Tick {
        tick(now, false, sig(mic, win), AppSignals::default())
    }

    fn prompts(events: &[Event]) -> Vec<App> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Prompted { app, .. } => Some(*app),
                _ => None,
            })
            .collect()
    }

    fn cleared(events: &[Event]) -> Vec<App> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Cleared { app } => Some(*app),
                _ => None,
            })
            .collect()
    }

    /// (time, prompted apps) for every tick that prompted.
    fn run(rules: &mut Rules, ticks: Vec<Tick>) -> Vec<(u64, Vec<App>)> {
        ticks
            .iter()
            .filter_map(|t| {
                let p = prompts(&rules.tick(t));
                (!p.is_empty()).then_some((t.now, p))
            })
            .collect()
    }

    #[test]
    fn zoom_prompts_once_when_mic_and_window_meet() {
        let mut r = Rules::new();
        let mut ticks = vec![zoom_only(0, false, false), zoom_only(2, true, false)];
        ticks.extend((4..=20).step_by(2).map(|t| zoom_only(t, true, true)));
        assert_eq!(run(&mut r, ticks), vec![(4, vec![App::Zoom])]);
    }

    #[test]
    fn home_window_only_never_prompts() {
        let mut r = Rules::new();
        let ticks = (0..=60).step_by(2).map(|t| zoom_only(t, true, false)).collect();
        assert!(run(&mut r, ticks).is_empty());
    }

    #[test]
    fn two_apps_two_prompts() {
        let mut r = Rules::new();
        let mut ticks = vec![zoom_only(0, false, false)];
        for t in (2..=10).step_by(2) {
            let teams = if t >= 6 { sig(true, true) } else { AppSignals::default() };
            ticks.push(tick(t, false, sig(true, true), teams));
        }
        assert_eq!(
            run(&mut r, ticks),
            vec![(2, vec![App::Zoom]), (6, vec![App::Teams])]
        );
    }

    #[test]
    fn call_seen_while_recording_is_never_prompted() {
        let mut r = Rules::new();
        let mut got = Vec::new();
        for t in (0..=20).step_by(2) {
            let zoom = if t >= 4 { sig(true, true) } else { AppSignals::default() };
            got.extend(prompts(&r.tick(&tick(t, t < 20, zoom, AppSignals::default()))));
        }
        assert!(got.is_empty(), "no prompt during or just after the recording");
        // Quiet t22, t24, t26: ended at t26, so t30 is a new call.
        for t in [22, 24, 26] {
            assert!(prompts(&r.tick(&zoom_only(t, false, false))).is_empty());
        }
        assert!(r.prompted().is_empty());
        assert_eq!(prompts(&r.tick(&zoom_only(30, true, true))), vec![App::Zoom]);
    }

    #[test]
    fn call_still_on_before_end_is_not_a_new_call() {
        // The same recording case, but Zoom only goes quiet for two ticks.
        let mut r = Rules::new();
        r.tick(&tick(0, true, sig(true, true), AppSignals::default()));
        r.tick(&zoom_only(2, false, false));
        r.tick(&zoom_only(4, false, false));
        assert!(prompts(&r.tick(&zoom_only(6, true, true))).is_empty());
    }

    #[test]
    fn dismissal_silences_the_app_for_thirty_minutes() {
        let mut r = Rules::new();
        for t in [2, 4] {
            r.tick(&zoom_only(t, true, true));
        }
        assert_eq!(r.prompted(), vec![(App::Zoom, None)]);
        // Dismissed at t10.
        r.tick(&zoom_only(6, true, true));
        r.tick(&zoom_only(8, true, true));
        assert_eq!(r.dismiss(App::Zoom, 10), Some(Event::Cleared { app: App::Zoom }));
        // Same call goes on to t18, then ends t20, t22, t24.
        assert!(prompts(&r.tick(&zoom_only(12, true, true))).is_empty());
        for t in [20, 22, 24] {
            r.tick(&zoom_only(t, false, false));
        }
        assert!(prompts(&r.tick(&zoom_only(40, true, true))).is_empty());
        for t in [42, 44, 46] {
            r.tick(&zoom_only(t, false, false));
        }
        // Teams is unaffected.
        let teams = tick(50, false, AppSignals::default(), sig(true, true));
        assert_eq!(prompts(&r.tick(&teams)), vec![App::Teams]);
        // A call at t10 + 1800 prompts.
        assert_eq!(prompts(&r.tick(&zoom_only(10 + 1800, true, true))), vec![App::Zoom]);
    }

    #[test]
    fn dismissal_just_before_thirty_minutes_still_silences() {
        let mut r = Rules::new();
        r.tick(&zoom_only(4, true, true));
        r.dismiss(App::Zoom, 10);
        for t in [12, 14, 16] {
            r.tick(&zoom_only(t, false, false));
        }
        assert!(prompts(&r.tick(&zoom_only(10 + 1798, true, true))).is_empty());
    }

    fn teams_plain(now: u64, mic: bool, frontmost: bool) -> Tick {
        let teams = AppSignals { mic, plain_window: true, frontmost, ..Default::default() };
        tick(now, false, AppSignals::default(), teams)
    }

    #[test]
    fn teams_plain_window_counts_when_fronted_soon_after_mic() {
        let mut r = Rules::new();
        let ticks = (0..=20).step_by(2).map(|t| teams_plain(t, true, t == 10)).collect();
        assert_eq!(run(&mut r, ticks), vec![(10, vec![App::Teams])]);
    }

    #[test]
    fn teams_plain_window_fronted_too_late_does_not_count() {
        let mut r = Rules::new();
        let ticks = (0..=60).step_by(2).map(|t| teams_plain(t, true, t == 40)).collect();
        assert!(run(&mut r, ticks).is_empty());
    }

    #[test]
    fn zoom_ignores_the_frontmost_rule() {
        let mut r = Rules::new();
        let zoom = AppSignals { mic: true, plain_window: true, frontmost: true, ..Default::default() };
        assert!(prompts(&r.tick(&tick(0, false, zoom, AppSignals::default()))).is_empty());
    }

    #[test]
    fn mic_off_for_two_ticks_with_the_window_open_keeps_one_prompt() {
        // Brief case 7: prompted at t4, only the mic drops at t10 and t12, the window stays on, mic back at t14.
        let mut r = Rules::new();
        let mut all = Vec::new();
        for (t, mic) in [(2, false), (4, true), (6, true), (8, true), (10, false), (12, false), (14, true), (16, true)] {
            all.extend(r.tick(&zoom_only(t, mic, t >= 4)));
        }
        assert_eq!(prompts(&all), vec![App::Zoom]);
        assert!(cleared(&all).is_empty());
        assert_eq!(r.prompted().len(), 1);
    }

    #[test]
    fn seen_call_with_open_window_survives_a_long_mic_gap_without_reprompt() {
        let mut r = Rules::new();
        let mut all = Vec::new();
        // Seen while recording, then the recording stops.
        for t in [2, 4] {
            all.extend(r.tick(&tick(t, true, sig(true, true), AppSignals::default())));
        }
        assert!(r.needs_windows(false, false), "an open call keeps the windows listed");
        // Mic off for four ticks, meeting window still open: the call does not end.
        for t in [6, 8, 10, 12] {
            all.extend(r.tick(&zoom_only(t, false, true)));
        }
        all.extend(r.tick(&zoom_only(14, true, true)));
        all.extend(r.tick(&zoom_only(16, true, true)));
        assert!(prompts(&all).is_empty(), "a seen call never prompts again: {:?}", all);
    }

    #[test]
    fn idle_mac_lists_no_windows() {
        assert!(!Rules::new().needs_windows(false, false));
    }

    #[test]
    fn recording_started_clears_a_prompt_and_marks_the_call_seen() {
        let mut r = Rules::new();
        r.tick(&zoom_only(2, true, true));
        assert_eq!(r.prompted().len(), 1);
        assert_eq!(cleared(&r.recording_started(None)), vec![App::Zoom]);
        assert!(r.prompted().is_empty());
        // Recording stops; the same call must not prompt again.
        let events = r.tick(&zoom_only(4, true, true));
        assert!(prompts(&events).is_empty());
    }

    #[test]
    fn short_mic_gap_neither_reprompts_nor_clears() {
        let mut r = Rules::new();
        let mut all = Vec::new();
        for (t, mic) in [(2, true), (4, true), (6, true), (8, true), (10, false), (12, false), (14, true)] {
            all.extend(r.tick(&zoom_only(t, mic, mic && t < 10 || t >= 14)));
        }
        assert_eq!(prompts(&all), vec![App::Zoom]);
        assert!(cleared(&all).is_empty());
    }

    #[test]
    fn mic_and_window_gap_of_two_ticks_keeps_the_call() {
        let mut r = Rules::new();
        r.tick(&zoom_only(4, true, true));
        r.tick(&zoom_only(10, false, false));
        let ev = r.tick(&zoom_only(12, false, false));
        assert!(ev.is_empty());
        assert!(r.tick(&zoom_only(14, true, true)).is_empty());
        assert_eq!(r.prompted().len(), 1);
    }

    #[test]
    fn recording_start_clears_prompts_and_blocks_reprompt() {
        let mut r = Rules::new();
        r.tick(&zoom_only(2, true, true));
        let ev = r.tick(&tick(8, true, sig(true, true), AppSignals::default()));
        assert_eq!(cleared(&ev), vec![App::Zoom]);
        assert!(r.prompted().is_empty());
        assert!(prompts(&r.tick(&zoom_only(30, true, true))).is_empty());
    }

    #[test]
    fn ended_prompted_call_emits_cleared_once() {
        let mut r = Rules::new();
        r.tick(&zoom_only(2, true, true));
        let mut all = Vec::new();
        for t in [4, 6, 8, 10] {
            all.extend(r.tick(&zoom_only(t, false, false)));
        }
        assert_eq!(cleared(&all), vec![App::Zoom]);
    }

    #[test]
    fn windows_are_listed_only_when_needed() {
        let mut r = Rules::new();
        assert!(!r.needs_windows(false, false));
        assert!(r.needs_windows(false, true));
        r.tick(&zoom_only(2, true, true));
        assert!(r.needs_windows(false, false), "a prompted call keeps listing");
    }

    #[test]
    fn prompt_carries_the_window_title() {
        let mut r = Rules::new();
        let zoom = AppSignals { title: Some("Standup".into()), ..sig(true, true) };
        let ev = r.tick(&tick(2, false, zoom, AppSignals::default()));
        assert_eq!(ev, vec![Event::Prompted { app: App::Zoom, title: Some("Standup".into()) }]);
        assert_eq!(r.prompted(), vec![(App::Zoom, Some("Standup".into()))]);
    }

    // ---- meeting-ended prompt ----

    fn ended(events: &[Event]) -> Vec<App> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Ended { app } => Some(*app),
                _ => None,
            })
            .collect()
    }

    fn ended_cleared(events: &[Event]) -> Vec<App> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::EndedCleared { app } => Some(*app),
                _ => None,
            })
            .collect()
    }

    /// A recording tick tied to `scope`.
    fn rec(now: u64, scope: Option<App>, zoom: AppSignals, teams: AppSignals) -> Tick {
        Tick { now, recording: true, recording_app: scope, zoom, teams }
    }

    fn quiet() -> AppSignals {
        AppSignals::default()
    }

    /// Feeds quiet recording ticks from `from`; returns every event raised.
    fn go_quiet(r: &mut Rules, from: u64, scope: Option<App>) -> Vec<Event> {
        (0..QUIET_TICKS_TO_END as u64)
            .flat_map(|i| r.tick(&rec(from + 2 * i, scope, quiet(), quiet())))
            .collect()
    }

    #[test]
    fn call_seen_during_recording_prompts_when_it_ends() {
        let mut r = Rules::new();
        for t in [0, 2, 4] {
            assert!(ended(&r.tick(&rec(t, None, sig(true, true), quiet()))).is_empty());
        }
        let ev = go_quiet(&mut r, 6, None);
        assert_eq!(ended(&ev), vec![App::Zoom]);
        assert_eq!(r.ended(), vec![App::Zoom]);
        // Once per end, not once per tick.
        assert!(ended(&r.tick(&rec(12, None, quiet(), quiet()))).is_empty());
    }

    #[test]
    fn call_open_before_the_recording_counts_once_it_is_recording() {
        let mut r = Rules::new();
        r.tick(&zoom_only(0, true, true));
        r.tick(&rec(2, None, sig(true, true), quiet()));
        assert_eq!(ended(&go_quiet(&mut r, 4, None)), vec![App::Zoom]);
    }

    #[test]
    fn recording_with_no_call_never_prompts() {
        let mut r = Rules::new();
        let mut all = Vec::new();
        for t in (0..=20).step_by(2) {
            all.extend(r.tick(&rec(t, None, quiet(), quiet())));
        }
        assert!(ended(&all).is_empty());
    }

    #[test]
    fn call_that_ends_before_the_recording_does_not_prompt() {
        let mut r = Rules::new();
        r.tick(&zoom_only(0, true, true));
        for t in [2, 4, 6] {
            r.tick(&zoom_only(t, false, false));
        }
        assert!(ended(&go_quiet(&mut r, 8, None)).is_empty());
    }

    #[test]
    fn call_that_ends_after_the_recording_stopped_does_not_prompt() {
        let mut r = Rules::new();
        r.tick(&rec(0, None, sig(true, true), quiet()));
        r.tick(&zoom_only(2, true, true));
        let ev: Vec<Event> = [4, 6, 8].iter().flat_map(|&t| r.tick(&zoom_only(t, false, false))).collect();
        assert!(ended(&ev).is_empty());
    }

    #[test]
    fn one_app_recording_ignores_the_other_apps_call() {
        let mut r = Rules::new();
        let scope = Some(App::Zoom);
        for t in [0, 2, 4] {
            r.tick(&rec(t, scope, quiet(), sig(true, true)));
        }
        assert!(ended(&go_quiet(&mut r, 6, scope)).is_empty());
        // And an open call of the other app does not hold back the watched app's prompt.
        let mut r = Rules::new();
        for t in [0, 2, 4] {
            r.tick(&rec(t, scope, sig(true, true), sig(true, true)));
        }
        let ev: Vec<Event> = [6, 8, 10].iter().flat_map(|&t| r.tick(&rec(t, scope, quiet(), sig(true, true)))).collect();
        assert_eq!(ended(&ev), vec![App::Zoom]);
    }

    #[test]
    fn no_prompt_while_another_tracked_call_is_open() {
        let mut r = Rules::new();
        for t in [0, 2, 4] {
            r.tick(&rec(t, None, sig(true, true), sig(true, true)));
        }
        // Zoom ends, Teams stays on: no prompt yet.
        let ev: Vec<Event> = [6, 8, 10].iter().flat_map(|&t| r.tick(&rec(t, None, quiet(), sig(true, true)))).collect();
        assert!(ended(&ev).is_empty());
        // Teams ends: its prompt shows.
        assert_eq!(ended(&go_quiet(&mut r, 12, None)), vec![App::Teams]);
    }

    #[test]
    fn two_calls_ending_on_the_same_tick_prompt_each_once() {
        let mut r = Rules::new();
        for t in [0, 2, 4] {
            r.tick(&rec(t, None, sig(true, true), sig(true, true)));
        }
        let ev = go_quiet(&mut r, 6, None);
        assert_eq!(ended(&ev), vec![App::Zoom, App::Teams]);
    }

    #[test]
    fn keep_recording_clears_and_a_later_call_prompts_again() {
        let mut r = Rules::new();
        r.tick(&rec(0, None, sig(true, true), quiet()));
        go_quiet(&mut r, 2, None);
        assert_eq!(r.keep(App::Zoom), Some(Event::EndedCleared { app: App::Zoom }));
        assert!(r.ended().is_empty());
        assert_eq!(r.keep(App::Zoom), None, "nothing left to keep");
        // The kept call stays quiet.
        assert!(ended(&r.tick(&rec(10, None, quiet(), quiet()))).is_empty());
        // A second call that starts and ends in the same recording prompts again.
        r.tick(&rec(12, None, sig(true, true), quiet()));
        assert_eq!(ended(&go_quiet(&mut r, 14, None)), vec![App::Zoom]);
    }

    #[test]
    fn rejoin_clears_the_ended_prompt() {
        let mut r = Rules::new();
        r.tick(&rec(0, None, sig(true, true), quiet()));
        go_quiet(&mut r, 2, None);
        let ev = r.tick(&rec(10, None, sig(true, true), quiet()));
        assert_eq!(ended_cleared(&ev), vec![App::Zoom]);
        assert!(r.ended().is_empty());
    }

    #[test]
    fn stopping_the_recording_clears_the_ended_prompt() {
        let mut r = Rules::new();
        r.tick(&rec(0, None, sig(true, true), quiet()));
        go_quiet(&mut r, 2, None);
        let ev = r.tick(&zoom_only(10, false, false));
        assert_eq!(ended_cleared(&ev), vec![App::Zoom]);
        assert!(r.ended().is_empty());
    }

    #[test]
    fn a_call_after_stop_start_prompts_and_never_ends_prompts() {
        let mut r = Rules::new();
        r.tick(&rec(0, None, sig(true, true), quiet()));
        go_quiet(&mut r, 2, None);
        r.tick(&zoom_only(10, false, false));
        assert_eq!(prompts(&r.tick(&zoom_only(12, true, true))), vec![App::Zoom]);
        assert!(r.ended().is_empty());
    }
}
