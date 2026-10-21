// assistant/meeting_name.rs
//
// Names a new meeting from the title of its Zoom or Teams window. Pure: the
// caller reads the titles from the frame store and applies the result.

use super::vision::CallApp;

const MAX_NAME_CHARS: usize = 120;

/// Titles that are only app chrome, compared case-insensitively.
const GENERIC: [&str; 6] = [
    "zoom",
    "zoom meeting",
    "zoom webinar",
    "zoom workplace",
    "meeting",
    "microsoft teams",
];

/// The first usable name among `titles` (first-seen order), or None when no
/// window title says anything about the meeting.
pub fn meeting_name_from_titles(titles: &[(CallApp, String)]) -> Option<String> {
    titles.iter().find_map(|(app, title)| clean_title(*app, title))
}

fn clean_title(app: CallApp, raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut name = collapsed.as_str();
    match app {
        CallApp::Zoom => {
            name = name.strip_suffix(" - Zoom").unwrap_or(name);
            for prefix in ["Zoom Meeting - ", "Zoom Webinar - "] {
                if let Some(rest) = name.strip_prefix(prefix) {
                    name = rest;
                    break;
                }
            }
        }
        CallApp::Teams => {
            name = name.strip_suffix(" | Microsoft Teams").unwrap_or(name);
        }
    }
    let name = name.trim();
    if name.is_empty() || GENERIC.iter().any(|g| name.eq_ignore_ascii_case(g)) {
        return None;
    }
    let capped: String = name.chars().take(MAX_NAME_CHARS).collect();
    Some(capped.trim().to_string())
}

/// True for the names a recording gets by itself: the frontend's
/// "Meeting DD_MM_YY_HH_MM_SS" and the backend's "Meeting YYYY-MM-DD_HH-MM-SS".
pub fn is_automatic_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("Meeting ") else {
        return false;
    };
    let shape = |pattern: &str| {
        rest.len() == pattern.len()
            && rest.chars().zip(pattern.chars()).all(|(c, p)| match p {
                '9' => c.is_ascii_digit(),
                other => c == other,
            })
    };
    shape("99_99_99_99_99_99") || shape("9999-99-99_99-99-99")
}

/// The name to save the meeting under: Some(new) when `current` is still the
/// automatic name and a window title gives a better one, None to keep
/// `current`.
pub fn renamed_from_titles(current: &str, titles: &[(CallApp, String)]) -> Option<String> {
    if !is_automatic_name(current) {
        return None;
    }
    meeting_name_from_titles(titles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(t: &str) -> (CallApp, String) {
        (CallApp::Zoom, t.to_string())
    }
    fn t(t: &str) -> (CallApp, String) {
        (CallApp::Teams, t.to_string())
    }

    #[test]
    fn zoom_title_kept_whole() {
        assert_eq!(
            meeting_name_from_titles(&[z("PNC Bank (Internal) : Joseph 1:1")]).as_deref(),
            Some("PNC Bank (Internal) : Joseph 1:1")
        );
    }

    #[test]
    fn zoom_suffix_stripped() {
        assert_eq!(
            meeting_name_from_titles(&[z("PNC Bank (Internal) : Joseph 1:1 - Zoom")]).as_deref(),
            Some("PNC Bank (Internal) : Joseph 1:1")
        );
    }

    #[test]
    fn zoom_prefixes_stripped() {
        assert_eq!(
            meeting_name_from_titles(&[z("Zoom Meeting - Budget sync")]).as_deref(),
            Some("Budget sync")
        );
        assert_eq!(
            meeting_name_from_titles(&[z("Zoom Webinar - Launch")]).as_deref(),
            Some("Launch")
        );
    }

    #[test]
    fn generic_zoom_title_is_none() {
        assert_eq!(meeting_name_from_titles(&[z("Zoom Meeting")]), None);
    }

    #[test]
    fn skips_generic_then_takes_teams_title() {
        assert_eq!(
            meeting_name_from_titles(&[z("zoom meeting"), t("Storage roadmap review | Microsoft Teams")])
                .as_deref(),
            Some("Storage roadmap review")
        );
    }

    #[test]
    fn generic_teams_title_is_none() {
        assert_eq!(meeting_name_from_titles(&[t("Microsoft Teams")]), None);
    }

    #[test]
    fn inner_whitespace_collapsed() {
        assert_eq!(
            meeting_name_from_titles(&[t("Meeting with Jim  Smith | Microsoft Teams")]).as_deref(),
            Some("Meeting with Jim Smith")
        );
    }

    #[test]
    fn no_titles_is_none() {
        assert_eq!(meeting_name_from_titles(&[]), None);
    }

    #[test]
    fn name_is_capped_at_a_char_boundary() {
        let long = "é".repeat(200);
        let name = meeting_name_from_titles(&[z(&long)]).unwrap();
        assert_eq!(name.chars().count(), 120);
    }

    #[test]
    fn automatic_names_recognised() {
        assert!(is_automatic_name("Meeting 21_10_26_10_04_31"));
        assert!(is_automatic_name("Meeting 2026-10-21_10-04-31"));
        assert!(!is_automatic_name("Budget sync"));
        assert!(!is_automatic_name("Meeting with Jim"));
        assert!(!is_automatic_name("Meeting 21_10_26_10_04_31 notes"));
    }

    #[test]
    fn automatic_name_is_replaced() {
        assert_eq!(
            renamed_from_titles(
                "Meeting 21_10_26_10_04_31",
                &[t("Storage roadmap review | Microsoft Teams")]
            )
            .as_deref(),
            Some("Storage roadmap review")
        );
    }

    #[test]
    fn typed_name_is_kept() {
        assert_eq!(
            renamed_from_titles("My own name", &[t("Storage roadmap review | Microsoft Teams")]),
            None
        );
    }

    #[test]
    fn automatic_name_stays_without_a_usable_title() {
        assert_eq!(renamed_from_titles("Meeting 21_10_26_10_04_31", &[z("Zoom Meeting")]), None);
    }
}
