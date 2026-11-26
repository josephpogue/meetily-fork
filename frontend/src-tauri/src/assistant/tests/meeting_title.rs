// Locked tests for assistant/meeting_title.rs, written by the tester.
// Entry points: suggest, name_to_apply, topic_from_reply, and
// crate::assistant::note::parse_note (TITLE section must not disturb it).

use super::*;
use std::io::Write;

const AUTO_OLD: &str = "Meeting 25_11_26_11_03_31";
const AUTO_ISO: &str = "Meeting 2026-10-21_10-04-31";
const WINDOW: &str = "PNC Teams meeting";
const TYPED: &str = "CSL - Maikol Intro / Catch Up Call 11/9/26";

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn codes_file(pairs: &[(&str, &str)]) -> tempfile::NamedTempFile {
    let body: Vec<String> = pairs.iter().map(|(k, v)| format!("\"{}\": \"{}\"", k, v)).collect();
    let json = format!(
        "{{\"_about\": \"Short project codes used in meeting titles.\", \"codes\": {{{}}}}}",
        body.join(", ")
    );
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(json.as_bytes()).unwrap();
    f
}

fn raw_file(text: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(text.as_bytes()).unwrap();
    f
}

fn slugs(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Title for one project through suggest, with a replaceable current name.
fn title_for(slug: &str, pairs: &[(&str, &str)], topic: &str, date: NaiveDate) -> String {
    let f = codes_file(pairs);
    let s = suggest(AUTO_OLD, None, Some(topic), date, &slugs(&[slug]), f.path());
    s.by_project.get(slug).cloned().expect("slug present in by_project")
}

// ---------- worked examples ----------

#[test]
fn example_pnc_workshop() {
    let t = title_for("broadcom-pnc-bank", &[("broadcom-pnc-bank", "PNC")], "VCF Design Workshop", d(2026, 10, 26));
    assert_eq!(t, "PNC - VCF Design Workshop 10/26/26");
}

#[test]
fn example_csl_sprint_planning() {
    let t = title_for("broadcom-csl-behring", &[("broadcom-csl-behring", "CSL")], "Sprint Planning Meeting", d(2026, 11, 5));
    assert_eq!(t, "CSL - Sprint Planning Meeting 11/5/26");
}

#[test]
fn example_collaborator_design_takes_parent_code() {
    let t = title_for("collaborator-design", &[("collaborator", "COL")], "Stage Review", d(2026, 1, 9));
    assert_eq!(t, "COL - Stage Review 1/9/26");
}

#[test]
fn example_wave_plan_takes_longest_prefix() {
    let t = title_for(
        "broadcom-pnc-bank-wave-plan",
        &[("broadcom", "BRCM"), ("broadcom-pnc-bank", "PNC")],
        "Wave Plan",
        d(2026, 12, 31),
    );
    assert_eq!(t, "PNC - Wave Plan 12/31/26");
}

#[test]
fn example_longest_prefix_independent_of_file_order() {
    let t = title_for(
        "broadcom-pnc-bank-wave-plan",
        &[("broadcom-pnc-bank", "PNC"), ("broadcom", "BRCM")],
        "Wave Plan",
        d(2026, 12, 31),
    );
    assert_eq!(t, "PNC - Wave Plan 12/31/26");
}

#[test]
fn example_job_search_no_entry_has_no_code() {
    let t = title_for("job-search", &[("broadcom-pnc-bank", "PNC")], "Recruiter Screen", d(2026, 11, 25));
    assert_eq!(t, "Recruiter Screen 11/25/26");
}

#[test]
fn example_no_project() {
    let f = codes_file(&[("broadcom-pnc-bank", "PNC")]);
    let s = suggest(AUTO_OLD, None, Some("PSO Automation Community Call"), d(2026, 11, 25), &slugs(&["broadcom-pnc-bank"]), f.path());
    assert_eq!(s.no_project, "PSO Automation Community Call 11/25/26");
}

#[test]
fn example_codes_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("project-codes.json");
    let s = suggest(AUTO_OLD, None, Some("Capacity Planning"), d(2026, 11, 25), &slugs(&["broadcom-pnc-bank"]), &missing);
    assert_eq!(s.by_project["broadcom-pnc-bank"], "Capacity Planning 11/25/26");
}

// ---------- codes edge cases ----------

#[test]
fn own_code_beats_parent_code() {
    let t = title_for(
        "broadcom-pnc-bank",
        &[("broadcom", "BRCM"), ("broadcom-pnc-bank", "PNC")],
        "Kickoff",
        d(2026, 11, 5),
    );
    assert_eq!(t, "PNC - Kickoff 11/5/26");
}

#[test]
fn sibling_with_similar_name_is_not_a_child() {
    // "broadcom-pnc-bankx" needs the "-" to count as a child of "broadcom-pnc-bank".
    let t = title_for("broadcom-pnc-bankx", &[("broadcom-pnc-bank", "PNC")], "Kickoff", d(2026, 11, 5));
    assert_eq!(t, "Kickoff 11/5/26");
}

#[test]
fn similar_name_still_takes_real_parent() {
    let t = title_for(
        "broadcom-pnc-bankx",
        &[("broadcom", "BRCM"), ("broadcom-pnc-bank", "PNC")],
        "Kickoff",
        d(2026, 11, 5),
    );
    assert_eq!(t, "BRCM - Kickoff 11/5/26");
}

#[test]
fn malformed_codes_file_gives_no_code() {
    let f = raw_file("{ this is not json");
    let s = suggest(AUTO_OLD, None, Some("Capacity Planning"), d(2026, 11, 25), &slugs(&["broadcom-pnc-bank"]), f.path());
    assert_eq!(s.by_project["broadcom-pnc-bank"], "Capacity Planning 11/25/26");
}

#[test]
fn codes_file_without_codes_object_gives_no_code() {
    let f = raw_file("{\"_about\": \"Short project codes.\"}");
    let s = suggest(AUTO_OLD, None, Some("Capacity Planning"), d(2026, 11, 25), &slugs(&["broadcom-pnc-bank"]), f.path());
    assert_eq!(s.by_project["broadcom-pnc-bank"], "Capacity Planning 11/25/26");
}

#[test]
fn empty_codes_file_gives_no_code() {
    let f = raw_file("");
    let s = suggest(AUTO_OLD, None, Some("Capacity Planning"), d(2026, 11, 25), &slugs(&["broadcom-pnc-bank"]), f.path());
    assert_eq!(s.by_project["broadcom-pnc-bank"], "Capacity Planning 11/25/26");
}

#[test]
fn unreadable_codes_path_gives_no_code() {
    // A directory cannot be read as a file.
    let dir = tempfile::tempdir().unwrap();
    let s = suggest(AUTO_OLD, None, Some("Capacity Planning"), d(2026, 11, 25), &slugs(&["broadcom-pnc-bank"]), dir.path());
    assert_eq!(s.by_project["broadcom-pnc-bank"], "Capacity Planning 11/25/26");
}

// ---------- dates ----------

#[test]
fn date_has_no_leading_zeros() {
    let f = codes_file(&[]);
    let a = suggest(AUTO_OLD, None, Some("Stage Review"), d(2026, 1, 9), &[], f.path());
    assert_eq!(a.no_project, "Stage Review 1/9/26");
    let b = suggest(AUTO_OLD, None, Some("Stage Review"), d(2026, 12, 31), &[], f.path());
    assert_eq!(b.no_project, "Stage Review 12/31/26");
    let c = suggest(AUTO_OLD, None, Some("Stage Review"), d(2026, 11, 5), &[], f.path());
    assert_eq!(c.no_project, "Stage Review 11/5/26");
}

#[test]
fn year_is_two_digits_across_years() {
    let f = codes_file(&[]);
    let s = suggest(AUTO_OLD, None, Some("Stage Review"), d(2027, 3, 10), &[], f.path());
    assert_eq!(s.no_project, "Stage Review 3/10/27");
}

// ---------- suggest: replaceable names ----------

#[test]
fn old_automatic_name_is_replaced() {
    let f = codes_file(&[("broadcom-pnc-bank", "PNC")]);
    let s = suggest(AUTO_OLD, None, Some("VCF Design Workshop"), d(2026, 10, 26), &slugs(&["broadcom-pnc-bank"]), f.path());
    assert!(!s.keep_current);
    assert_eq!(s.current, AUTO_OLD);
    assert_eq!(s.no_project, "VCF Design Workshop 10/26/26");
}

#[test]
fn iso_automatic_name_is_replaced() {
    let f = codes_file(&[("broadcom-pnc-bank", "PNC")]);
    let s = suggest(AUTO_ISO, None, Some("VCF Design Workshop"), d(2026, 10, 26), &slugs(&["broadcom-pnc-bank"]), f.path());
    assert!(!s.keep_current);
    assert_eq!(s.current, AUTO_ISO);
    assert_eq!(s.by_project["broadcom-pnc-bank"], "PNC - VCF Design Workshop 10/26/26");
}

#[test]
fn window_name_is_replaced() {
    let f = codes_file(&[("broadcom-pnc-bank", "PNC")]);
    let s = suggest(WINDOW, Some(WINDOW), Some("VCF Design Workshop"), d(2026, 10, 26), &slugs(&["broadcom-pnc-bank"]), f.path());
    assert!(!s.keep_current);
    assert_eq!(s.current, WINDOW);
    assert_eq!(s.by_project["broadcom-pnc-bank"], "PNC - VCF Design Workshop 10/26/26");
}

#[test]
fn window_name_with_one_to_one_tail_is_replaced() {
    let name = "PNC Bank (Internal) : Joseph 1:1";
    let f = codes_file(&[]);
    let s = suggest(name, Some(name), Some("Weekly Sync"), d(2026, 11, 25), &[], f.path());
    assert!(!s.keep_current);
    assert_eq!(s.no_project, "Weekly Sync 11/25/26");
}

#[test]
fn typed_name_is_kept_when_window_name_differs() {
    let f = codes_file(&[("broadcom-csl-behring", "CSL")]);
    let s = suggest(TYPED, Some(WINDOW), Some("Intro Call"), d(2026, 11, 9), &slugs(&["broadcom-csl-behring", "job-search"]), f.path());
    assert!(s.keep_current);
    assert_eq!(s.current, TYPED);
    assert_eq!(s.no_project, TYPED);
    assert_eq!(s.by_project["broadcom-csl-behring"], TYPED);
    assert_eq!(s.by_project["job-search"], TYPED);
}

#[test]
fn typed_name_is_kept_when_window_name_is_none() {
    let f = codes_file(&[]);
    let s = suggest(TYPED, None, Some("Intro Call"), d(2026, 11, 9), &slugs(&["job-search"]), f.path());
    assert!(s.keep_current);
    assert_eq!(s.no_project, TYPED);
    assert_eq!(s.by_project["job-search"], TYPED);
}

#[test]
fn name_starting_with_meeting_but_not_a_timestamp_is_typed() {
    let f = codes_file(&[]);
    let s = suggest("Meeting Notes", None, Some("Intro Call"), d(2026, 11, 9), &[], f.path());
    assert!(s.keep_current);
    assert_eq!(s.no_project, "Meeting Notes");
}

#[test]
fn no_topic_keeps_current_even_for_automatic_name() {
    let f = codes_file(&[("broadcom-pnc-bank", "PNC")]);
    let s = suggest(AUTO_OLD, None, None, d(2026, 10, 26), &slugs(&["broadcom-pnc-bank", "job-search"]), f.path());
    assert!(s.keep_current);
    assert_eq!(s.current, AUTO_OLD);
    assert_eq!(s.no_project, AUTO_OLD);
    assert_eq!(s.by_project["broadcom-pnc-bank"], AUTO_OLD);
    assert_eq!(s.by_project["job-search"], AUTO_OLD);
}

#[test]
fn no_topic_keeps_current_for_window_name() {
    let f = codes_file(&[]);
    let s = suggest(WINDOW, Some(WINDOW), None, d(2026, 10, 26), &[], f.path());
    assert!(s.keep_current);
    assert_eq!(s.no_project, WINDOW);
}

#[test]
fn by_project_has_exactly_the_listed_slugs() {
    let f = codes_file(&[("collaborator", "COL"), ("broadcom-pnc-bank", "PNC"), ("broadcom", "BRCM")]);
    let list = slugs(&["broadcom", "collaborator", "collaborator-design", "job-search"]);
    let s = suggest(AUTO_OLD, None, Some("Stage Review"), d(2026, 1, 9), &list, f.path());
    let keys: Vec<&String> = s.by_project.keys().collect();
    let mut want = list.clone();
    want.sort();
    assert_eq!(keys.len(), 4);
    for (k, w) in keys.iter().zip(want.iter()) {
        assert_eq!(*k, w);
    }
    assert_eq!(s.by_project["broadcom"], "BRCM - Stage Review 1/9/26");
    assert_eq!(s.by_project["collaborator"], "COL - Stage Review 1/9/26");
    assert_eq!(s.by_project["collaborator-design"], "COL - Stage Review 1/9/26");
    assert_eq!(s.by_project["job-search"], "Stage Review 1/9/26");
}

#[test]
fn empty_project_list_gives_empty_by_project() {
    let f = codes_file(&[("broadcom-pnc-bank", "PNC")]);
    let s = suggest(AUTO_OLD, None, Some("Stage Review"), d(2026, 1, 9), &[], f.path());
    assert!(s.by_project.is_empty());
    assert_eq!(s.no_project, "Stage Review 1/9/26");
}

// ---------- topic_from_reply ----------

fn reply(slug: &str, title: Option<&str>, project: &str, note: &str) -> String {
    let mut out = format!("=== SLUG ===\n{}\n", slug);
    if let Some(t) = title {
        out.push_str(&format!("=== TITLE ===\n{}\n", t));
    }
    out.push_str(&format!("=== PROJECT ===\n{}\n=== NOTE ===\n{}\n", project, note));
    out
}

const NOTE_BODY: &str = "# VCF Design Workshop\n\n## Decisions\n- Use two workload domains.\n\n## Action items\n- Joseph to send the sizing sheet.\n";

#[test]
fn topic_is_read_from_title_section() {
    let r = reply("2026-10-26-pnc-vcf-design-workshop", Some("VCF Design Workshop"), "broadcom-pnc-bank", NOTE_BODY);
    assert_eq!(topic_from_reply(&r), Some("VCF Design Workshop".to_string()));
}

#[test]
fn topic_collapses_newlines_and_runs_of_spaces() {
    let r = reply("2026-10-26-pnc-vcf-design-workshop", Some("  VCF   Design\n  Workshop \n"), "broadcom-pnc-bank", NOTE_BODY);
    assert_eq!(topic_from_reply(&r), Some("VCF Design Workshop".to_string()));
}

#[test]
fn topic_none_when_section_missing() {
    let r = reply("2026-10-26-pnc-vcf-design-workshop", None, "broadcom-pnc-bank", NOTE_BODY);
    assert_eq!(topic_from_reply(&r), None);
}

#[test]
fn topic_none_when_section_empty() {
    let r = reply("2026-10-26-pnc-vcf-design-workshop", Some(""), "broadcom-pnc-bank", NOTE_BODY);
    assert_eq!(topic_from_reply(&r), None);
}

#[test]
fn topic_none_when_section_whitespace_only() {
    let r = reply("2026-10-26-pnc-vcf-design-workshop", Some("  \n \t \n"), "broadcom-pnc-bank", NOTE_BODY);
    assert_eq!(topic_from_reply(&r), None);
}

#[test]
fn topic_does_not_swallow_the_next_section() {
    let r = reply("2026-10-26-pnc-vcf-design-workshop", Some("Stage Review"), "collaborator-design", NOTE_BODY);
    let t = topic_from_reply(&r).unwrap();
    assert_eq!(t, "Stage Review");
    assert!(!t.contains("collaborator-design"));
    assert!(!t.contains("PROJECT"));
}

// ---------- parse_note with and without the TITLE section ----------

#[test]
fn parse_note_same_with_or_without_title() {
    let slug = "2026-10-26-pnc-vcf-design-workshop";
    let with = crate::assistant::note::parse_note(&reply(slug, Some("VCF Design Workshop"), "broadcom-pnc-bank", NOTE_BODY)).expect("with title parses");
    let without = crate::assistant::note::parse_note(&reply(slug, None, "broadcom-pnc-bank", NOTE_BODY)).expect("old reply parses");
    assert_eq!(with.slug, slug);
    assert_eq!(with.slug, without.slug);
    assert_eq!(with.project, Some("broadcom-pnc-bank".to_string()));
    assert_eq!(with.project, without.project);
    assert_eq!(with.markdown, without.markdown);
    assert!(with.markdown.contains("Use two workload domains."));
}

#[test]
fn parse_note_title_text_does_not_leak() {
    let slug = "2026-11-05-csl-sprint-planning";
    let title = "Zebra Quokka Sprint Planning Meeting";
    let d = crate::assistant::note::parse_note(&reply(slug, Some(title), "broadcom-csl-behring", NOTE_BODY)).unwrap();
    assert!(!d.slug.contains("Zebra"));
    assert!(!d.project.clone().unwrap_or_default().contains("Zebra"));
    assert!(!d.markdown.contains("Zebra Quokka"));
    assert_eq!(d.project, Some("broadcom-csl-behring".to_string()));
}

#[test]
fn parse_note_multi_line_title_does_not_leak() {
    let slug = "2026-11-05-collaborator-stage-review";
    let title = "Stage Review\nwith a second line Quokka";
    let d = crate::assistant::note::parse_note(&reply(slug, Some(title), "collaborator-design", NOTE_BODY)).unwrap();
    assert_eq!(d.slug, slug);
    assert_eq!(d.project, Some("collaborator-design".to_string()));
    assert!(!d.markdown.contains("Quokka"));
}

#[test]
fn parse_note_empty_title_section_same_as_missing() {
    let slug = "2026-11-25-job-search-recruiter-screen";
    let a = crate::assistant::note::parse_note(&reply(slug, Some(""), "job-search", NOTE_BODY)).unwrap();
    let b = crate::assistant::note::parse_note(&reply(slug, None, "job-search", NOTE_BODY)).unwrap();
    assert_eq!(a.slug, b.slug);
    assert_eq!(a.project, b.project);
    assert_eq!(a.markdown, b.markdown);
}

// ---------- name_to_apply ----------

fn sugg(keep: bool) -> TitleSuggestions {
    let mut by = BTreeMap::new();
    by.insert("broadcom-pnc-bank".to_string(), "PNC - VCF Design Workshop 10/26/26".to_string());
    TitleSuggestions {
        by_project: by,
        no_project: "VCF Design Workshop 10/26/26".to_string(),
        current: AUTO_OLD.to_string(),
        keep_current: keep,
    }
}

#[test]
fn apply_empty_box_is_none() {
    assert_eq!(name_to_apply(&sugg(false), "", false, AUTO_OLD), None);
    assert_eq!(name_to_apply(&sugg(false), "", true, AUTO_OLD), None);
}

#[test]
fn apply_whitespace_box_is_none() {
    assert_eq!(name_to_apply(&sugg(false), "  \n\t ", false, AUTO_OLD), None);
    assert_eq!(name_to_apply(&sugg(false), "   ", true, AUTO_OLD), None);
}

#[test]
fn apply_edited_returns_trimmed_box() {
    let got = name_to_apply(&sugg(false), "  PNC - Wave Plan 12/31/26 \n", true, AUTO_OLD);
    assert_eq!(got, Some("PNC - Wave Plan 12/31/26".to_string()));
}

#[test]
fn apply_edited_wins_even_when_keep_current() {
    let got = name_to_apply(&sugg(true), "PNC - Wave Plan 12/31/26", true, AUTO_OLD);
    assert_eq!(got, Some("PNC - Wave Plan 12/31/26".to_string()));
}

#[test]
fn apply_edited_wins_even_when_sidebar_renamed() {
    let got = name_to_apply(&sugg(false), "COL - Stage Review 1/9/26", true, TYPED);
    assert_eq!(got, Some("COL - Stage Review 1/9/26".to_string()));
}

#[test]
fn apply_edited_equal_to_current_now_is_none() {
    assert_eq!(name_to_apply(&sugg(false), TYPED, true, TYPED), None);
}

#[test]
fn apply_edited_equal_to_current_now_after_trim_is_none() {
    assert_eq!(name_to_apply(&sugg(false), "  PNC Teams meeting ", true, "PNC Teams meeting"), None);
}

#[test]
fn apply_unedited_keep_current_is_none() {
    assert_eq!(name_to_apply(&sugg(true), AUTO_OLD, false, AUTO_OLD), None);
}

#[test]
fn apply_unedited_sidebar_rename_is_none() {
    let got = name_to_apply(&sugg(false), "PNC - VCF Design Workshop 10/26/26", false, TYPED);
    assert_eq!(got, None);
}

#[test]
fn apply_unedited_suggestion_is_applied() {
    let got = name_to_apply(&sugg(false), "PNC - VCF Design Workshop 10/26/26", false, AUTO_OLD);
    assert_eq!(got, Some("PNC - VCF Design Workshop 10/26/26".to_string()));
}

#[test]
fn apply_unedited_no_project_suggestion_is_applied() {
    let got = name_to_apply(&sugg(false), "VCF Design Workshop 10/26/26", false, AUTO_OLD);
    assert_eq!(got, Some("VCF Design Workshop 10/26/26".to_string()));
}
