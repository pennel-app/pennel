use crate::meetings::{MeetingStore, Segment, Speaker, SpeakerName, timestamp};
use crate::{InterfaceLanguage, ui_text};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct MeetingExport {
    pub title: String,
    pub markdown: String,
    pub html: String,
}

fn meta_line(store: &MeetingStore, id: &str, start_iso: &str, duration_min: u32) -> String {
    let mut parts = Vec::new();
    if let Ok(start) = chrono::DateTime::parse_from_rfc3339(start_iso) {
        parts.push(start.format("%d-%m-%Y %H:%M").to_string());
    }
    if duration_min > 0 {
        parts.push(format!("{duration_min} min"));
    }
    let attendees = store.attendees(id);
    if !attendees.is_empty() {
        parts.push(attendees.join(", "));
    }
    parts.join(" · ")
}

#[cfg(test)]
fn export_markdown(
    store: &MeetingStore,
    id: &str,
    include_transcript: bool,
) -> Option<(String, String)> {
    export_markdown_for_language(store, id, include_transcript, InterfaceLanguage::English)
}

pub(crate) fn exported_speaker_label(
    segment: &Segment,
    names: &[SpeakerName],
    language: InterfaceLanguage,
) -> String {
    match (segment.speaker, segment.voice) {
        (Speaker::Me, _) => ui_text(language, "meetings.speaker_me", &[]),
        (Speaker::Others, Some(voice)) => {
            names.iter().find(|name| name.voice == voice).map_or_else(
                || {
                    ui_text(
                        language,
                        "meetings.speaker_number",
                        &[("number", &voice.to_string())],
                    )
                },
                |name| name.name.clone(),
            )
        }
        (Speaker::Others, None) => ui_text(language, "meetings.speaker_others", &[]),
    }
}

pub fn export_markdown_for_language(
    store: &MeetingStore,
    id: &str,
    include_transcript: bool,
    language: InterfaceLanguage,
) -> Option<(String, String)> {
    let info = store.info(id)?;
    let mut out = format!("# {}\n", info.title);
    let meta = meta_line(store, id, &info.start_iso, info.duration_min);
    if !meta.is_empty() {
        out.push_str(&format!("\n{meta}\n"));
    }
    let body = store.read_meeting(id).unwrap_or_default();
    if !body.trim().is_empty() {
        out.push_str(&format!("\n{}\n", body.trim()));
    }
    let notes = store.read_notes(id);
    if !notes.trim().is_empty() {
        let heading = ui_text(language, "meetings.your_notes", &[]);
        out.push_str(&format!("\n## {heading}\n\n{}\n", notes.trim()));
    }
    if include_transcript {
        let names = store.speaker_names(id);
        let snapshot = store.read_transcript_snapshot(id).ok()?;
        let legacy = snapshot.metadata_status == crate::TranscriptMetadataStatus::Absent;
        let lines: Vec<String> = snapshot
            .utterances
            .into_iter()
            .filter(|row| !row.segment.failed && !row.segment.text.trim().is_empty())
            .map(|row| {
                let label = if legacy {
                    exported_speaker_label(&row.segment, &names, language)
                } else {
                    crate::meeting_evidence::localized_utterance_speaker_label(
                        &row, &names, language,
                    )
                };
                format!(
                    "**[{}] {}:** {}",
                    timestamp(row.segment.start_ms),
                    label,
                    row.segment.text.trim()
                )
            })
            .collect();
        if !lines.is_empty() {
            let heading = ui_text(language, "meetings.transcript", &[]);
            out.push_str(&format!("\n## {heading}\n\n{}\n", lines.join("\n\n")));
        }
    }
    Some((info.title, out))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn emphasis_tags(run: usize) -> (&'static str, &'static str) {
    match run {
        1 => ("<em>", "</em>"),
        2 => ("<strong>", "</strong>"),
        _ => ("<strong><em>", "</em></strong>"),
    }
}

/// `*`, `**` and `***` runs pair only when they hug text (`5 * 3` stays literal); unpaired runs stay literal.
fn emphasis(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '*' {
            plain.push(chars[i]);
            i += 1;
            continue;
        }
        let run = chars[i..].iter().take_while(|c| **c == '*').count();
        let before = i.checked_sub(1).map(|p| chars[p]);
        let after = chars.get(i + run).copied();
        let can_close = before.is_some_and(|c| !c.is_whitespace());
        let can_open = after.is_some_and(|c| !c.is_whitespace());
        tokens.push(std::mem::take(&mut plain));
        let matching = open.iter().rposition(|(_, len)| *len == run);
        if let (true, Some(position)) = (can_close, matching) {
            let (index, _) = open[position];
            open.truncate(position);
            let (start, end) = emphasis_tags(run);
            tokens[index] = start.to_owned();
            tokens.push(end.to_owned());
        } else {
            if can_open && run <= 3 {
                open.push((tokens.len(), run));
            }
            tokens.push("*".repeat(run));
        }
        i += run;
    }
    tokens.push(plain);
    tokens.concat()
}

/// Links first, so emphasis never rewrites a URL; only http(s) and mailto become anchors.
fn rich(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find("](") else { break };
        let Some(end) = after[close + 2..].find(')') else {
            break;
        };
        let label = &after[..close];
        let url = &after[close + 2..close + 2 + end];
        out.push_str(&emphasis(&escape(&rest[..open])));
        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
            out.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                escape(url),
                emphasis(&escape(label))
            ));
        } else {
            out.push_str(&emphasis(&escape(&format!("[{label}]({url})"))));
        }
        rest = &after[close + 2 + end + 1..];
    }
    out.push_str(&emphasis(&escape(rest)));
    out
}

/// Code spans are cut out first and rendered verbatim; an unmatched backtick stays literal.
fn inline(text: &str) -> String {
    let parts: Vec<&str> = text.split('`').collect();
    let balanced = if parts.len() % 2 == 1 {
        parts.len()
    } else {
        parts.len() - 1
    };
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i >= balanced {
            out.push('`');
            out.push_str(&rich(part));
        } else if i % 2 == 1 {
            out.push_str(&format!("<code>{}</code>", escape(part)));
        } else {
            out.push_str(&rich(part));
        }
    }
    out
}

fn list_item(line: &str) -> Option<(&'static str, &str)> {
    if let Some(item) = ["- ", "* "].iter().find_map(|m| line.strip_prefix(m)) {
        return Some(("ul", item));
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    (digits > 0)
        .then(|| line[digits..].strip_prefix(". "))
        .flatten()
        .map(|item| ("ol", item))
}

fn item_html(item: &str) -> String {
    if let Some(rest) = item.strip_prefix("[ ] ") {
        format!("☐ {}", inline(rest))
    } else if let Some(rest) = item
        .strip_prefix("[x] ")
        .or_else(|| item.strip_prefix("[X] "))
    {
        format!("☑ {}", inline(rest))
    } else {
        inline(item)
    }
}

pub fn markdown_to_html(markdown: &str) -> String {
    let mut html = String::new();
    let mut list: Option<&'static str> = None;
    let mut paragraph: Vec<String> = Vec::new();
    let flush = |html: &mut String, paragraph: &mut Vec<String>| {
        if !paragraph.is_empty() {
            html.push_str(&format!("<p>{}</p>\n", paragraph.join("<br>")));
            paragraph.clear();
        }
    };
    for line in markdown.lines() {
        let trimmed = line.trim();
        let item = list_item(trimmed);
        if let Some(kind) = list
            && item.is_none_or(|(next, _)| next != kind)
        {
            html.push_str(&format!("</{kind}>\n"));
            list = None;
        }
        if trimmed.is_empty() {
            flush(&mut html, &mut paragraph);
        } else if let Some((kind, item)) = item {
            flush(&mut html, &mut paragraph);
            if list.is_none() {
                html.push_str(&format!("<{kind}>\n"));
                list = Some(kind);
            }
            html.push_str(&format!("<li>{}</li>\n", item_html(item)));
        } else if let Some(level) = (1..=3)
            .rev()
            .find(|n| trimmed.starts_with(&format!("{} ", "#".repeat(*n))))
        {
            flush(&mut html, &mut paragraph);
            html.push_str(&format!(
                "<h{level}>{}</h{level}>\n",
                inline(trimmed[level + 1..].trim())
            ));
        } else {
            paragraph.push(inline(trimmed));
        }
    }
    if let Some(kind) = list {
        html.push_str(&format!("</{kind}>\n"));
    }
    flush(&mut html, &mut paragraph);
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::{Segment, Speaker};
    use chrono::TimeZone;

    fn store() -> (tempfile::TempDir, MeetingStore, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store
            .create(
                "Weekly <sync> & co",
                chrono::Local
                    .with_ymd_and_hms(2026, 10, 20, 9, 30, 0)
                    .unwrap(),
            )
            .unwrap();
        (dir, store, id)
    }

    fn segment(ms: u64, speaker: Speaker, text: &str, failed: bool) -> Segment {
        Segment {
            start_ms: ms,
            end_ms: ms + 1000,
            speaker,
            text: text.into(),
            failed,
            voice: None,
        }
    }

    #[test]
    fn export_has_title_meta_and_summary() {
        let (_dir, store, id) = store();
        store
            .write_meeting(&id, 25, "algemeen", "## Samenvatting\n- Budget rond")
            .unwrap();
        store
            .set_attendees(&id, &["Jan".into(), "Els".into()])
            .unwrap();
        let (title, md) = export_markdown(&store, &id, false).unwrap();
        assert_eq!(title, "Weekly <sync> & co");
        assert_eq!(
            md,
            "# Weekly <sync> & co\n\n20-10-2026 09:30 · 25 min · Jan, Els\n\n## Samenvatting\n- Budget rond\n"
        );
    }

    #[test]
    fn export_without_summary_uses_notes_or_nothing() {
        let (_dir, store, id) = store();
        let (_, bare) = export_markdown(&store, &id, true).unwrap();
        assert_eq!(bare, "# Weekly <sync> & co\n\n20-10-2026 09:30\n");
        store.write_notes(&id, "- mijn notitie").unwrap();
        assert!(
            export_markdown(&store, &id, false)
                .unwrap()
                .1
                .ends_with("\n- mijn notitie\n")
        );
    }

    #[test]
    fn personal_export_keeps_notes_after_the_summary_in_each_interface_language() {
        let (_dir, store, id) = store();
        store
            .write_meeting(&id, 5, "algemeen", "## Besluit\nDe afspraak staat.")
            .unwrap();
        store
            .write_notes(&id, "PRIVATE_OWNER_NOTE <literal> & unchanged")
            .unwrap();
        store
            .write_transcript(&id, &[segment(0, Speaker::Me, "Transcript source", false)])
            .unwrap();
        for (language, heading) in [
            (InterfaceLanguage::English, "Your notes"),
            (InterfaceLanguage::Dutch, "Je notities"),
        ] {
            for include_transcript in [false, true] {
                let (_, output) =
                    export_markdown_for_language(&store, &id, include_transcript, language)
                        .unwrap();
                let notes = format!("## {heading}\n\nPRIVATE_OWNER_NOTE <literal> & unchanged");
                assert!(
                    output.contains(&notes),
                    "Personal export lost its notes: {output}"
                );
                assert!(output.find("De afspraak staat.").unwrap() < output.find(&notes).unwrap());
                assert_eq!(output.matches("PRIVATE_OWNER_NOTE").count(), 1);
                if include_transcript {
                    assert!(output.find(&notes).unwrap() < output.find("## Transcript").unwrap());
                }
                let html = markdown_to_html(&output);
                assert!(html.contains("&lt;literal&gt; &amp; unchanged"));
                assert!(!html.contains("<literal>"));
            }
        }
    }

    #[test]
    fn frontmatter_not_exported_and_failed_segments_skipped() {
        let (_dir, store, id) = store();
        store.write_meeting(&id, 5, "algemeen", "Kort.").unwrap();
        store
            .write_transcript(
                &id,
                &[
                    segment(0, Speaker::Me, "Hallo", false),
                    segment(65_000, Speaker::Others, "", true),
                ],
            )
            .unwrap();
        let (_, md) = export_markdown(&store, &id, true).unwrap();
        assert!(!md.contains("---") && !md.contains("template:"));
        assert!(md.ends_with("## Transcript\n\n**[00:00] Unknown speaker:** Hallo\n"));
    }

    #[test]
    fn localized_export_changes_only_owned_labels_and_keeps_saved_markers() {
        let (_dir, store, id) = store();
        let mut other = segment(1_000, Speaker::Others, "Mijn naam is Ik", false);
        other.voice = Some(2);
        store
            .write_transcript(
                &id,
                &[segment(0, Speaker::Me, "Mijn notitie", false), other],
            )
            .unwrap();
        store
            .write_notes(&id, "## Samenvatting\nMijn eigen tekst.")
            .unwrap();
        let saved = std::fs::read_to_string(store.dir_of(&id).join("transcript.md")).unwrap();
        let english =
            export_markdown_for_language(&store, &id, true, crate::InterfaceLanguage::English)
                .unwrap()
                .1;
        let dutch =
            export_markdown_for_language(&store, &id, true, crate::InterfaceLanguage::Dutch)
                .unwrap()
                .1;
        assert!(english.contains("Unknown speaker:** Mijn notitie"));
        assert!(english.contains("Speaker 2:** Mijn naam is Ik"));
        assert!(dutch.contains("Onbekende spreker:** Mijn notitie"));
        assert!(dutch.contains("Anderen 2:** Mijn naam is Ik"));
        assert!(english.contains("## Samenvatting\nMijn eigen tekst."));
        assert_eq!(
            std::fs::read_to_string(store.dir_of(&id).join("transcript.md")).unwrap(),
            saved
        );
        store
            .set_speaker_names(
                &id,
                &[SpeakerName {
                    voice: 2,
                    name: "Mijn eigen naam".into(),
                }],
            )
            .unwrap();
        for language in [InterfaceLanguage::English, InterfaceLanguage::Dutch] {
            let body = export_markdown_for_language(&store, &id, true, language)
                .unwrap()
                .1;
            assert!(body.contains("Mijn eigen naam:** Mijn naam is Ik"));
        }
    }

    #[test]
    fn canonical_export_localizes_uncertainty_without_inventing_the_user_or_translating_names() {
        let (_dir, store, id) = store();
        let mut rows = [
            "Room commitment",
            "Rapid turns",
            "Shared speech",
            "Named statement",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            crate::MeetingUtterance::new(
                segment(index as u64 * 1_000, Speaker::Me, text, false),
                Some("microphone".into()),
            )
        })
        .collect::<Vec<_>>();
        rows[1].metadata.attribution.kind = crate::AttributionKind::Mixed;
        rows[1].metadata.attribution.voices = vec![1, 2];
        rows[2].metadata.attribution.kind = crate::AttributionKind::Overlap;
        rows[2].metadata.attribution.voices = vec![1, 2];
        rows[3].metadata.manual = Some(crate::ManualCorrection {
            self_voice: false,
            name: Some("Me".into()),
            voice: Some(3),
            source_fingerprint: rows[3].metadata.source_fingerprint.clone(),
            review_required: false,
        });
        store.write_transcript_snapshot(&id, None, &rows).unwrap();
        let english = export_markdown_for_language(&store, &id, true, InterfaceLanguage::English)
            .unwrap()
            .1;
        let dutch = export_markdown_for_language(&store, &id, true, InterfaceLanguage::Dutch)
            .unwrap()
            .1;
        assert!(english.contains("Unknown speaker:** Room commitment"));
        assert!(english.contains("Mixed speakers:** Rapid turns"));
        assert!(english.contains("Overlapping speakers:** Shared speech"));
        assert!(dutch.contains("Onbekende spreker:** Room commitment"));
        assert!(dutch.contains("Meerdere sprekers:** Rapid turns"));
        assert!(dutch.contains("Overlappende sprekers:** Shared speech"));
        assert!(english.contains("Me:** Named statement"));
        assert!(dutch.contains("Me:** Named statement"));
        assert!(!english.contains("Me:** Room commitment"));
        assert!(!dutch.contains("Ik:** Room commitment"));
    }

    #[test]
    fn exported_live_estimates_are_localized_and_manual_confirmation_stays_confirmed() {
        let (_dir, store, id) = store();
        let mut row = crate::MeetingUtterance::new(
            segment(0, Speaker::Me, "Finalized speech", false),
            Some("microphone".into()),
        );
        row.metadata.attribution.kind = crate::AttributionKind::Single;
        row.metadata.attribution.voices = vec![1];
        row.metadata.attribution.provisional = true;
        store
            .write_transcript_snapshot(&id, None, std::slice::from_ref(&row))
            .unwrap();
        let english = export_markdown_for_language(&store, &id, true, InterfaceLanguage::English)
            .unwrap()
            .1;
        let dutch = export_markdown_for_language(&store, &id, true, InterfaceLanguage::Dutch)
            .unwrap()
            .1;
        assert!(english.contains("Speaker 1 (Provisional):** Finalized speech"));
        assert!(dutch.contains("Anderen 1 (Voorlopig):** Finalized speech"));
        row.metadata.manual = Some(crate::ManualCorrection {
            self_voice: false,
            name: Some("Ada".into()),
            voice: None,
            source_fingerprint: row.metadata.source_fingerprint.clone(),
            review_required: false,
        });
        store
            .write_transcript_snapshot(&id, None, std::slice::from_ref(&row))
            .unwrap();
        for language in [InterfaceLanguage::English, InterfaceLanguage::Dutch] {
            let exported = export_markdown_for_language(&store, &id, true, language)
                .unwrap()
                .1;
            assert!(exported.contains("Ada:** Finalized speech"));
            assert!(!exported.contains("(Provisional)"));
            assert!(!exported.contains("(Voorlopig)"));
        }
    }

    #[test]
    fn legacy_absent_metadata_exports_preserve_saved_labels_without_creating_a_sidecar() {
        let (_dir, store, id) = store();
        std::fs::write(
            store.dir_of(&id).join("transcript.md"),
            "[00:00] **Ik:** Hallo\n[00:02] **Anderen 2:** Een antwoord\n",
        )
        .unwrap();
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().metadata_status,
            crate::TranscriptMetadataStatus::Absent
        );
        let english = export_markdown_for_language(&store, &id, true, InterfaceLanguage::English)
            .unwrap()
            .1;
        let dutch = export_markdown_for_language(&store, &id, true, InterfaceLanguage::Dutch)
            .unwrap()
            .1;
        assert!(english.contains("Me:** Hallo"));
        assert!(english.contains("Speaker 2:** Een antwoord"));
        assert!(dutch.contains("Ik:** Hallo"));
        assert!(dutch.contains("Anderen 2:** Een antwoord"));
        assert!(!store.dir_of(&id).join("transcript.meta.json").exists());
    }

    #[test]
    fn markdown_to_html_lists_and_headings() {
        let html = markdown_to_html(
            "# Titel\n\n## Actiepunten\n- [ ] Jan — **offerte**\n- [x] klaar\n\nTekst *schuin* en `code`\nregel twee",
        );
        assert_eq!(
            html,
            "<h1>Titel</h1>\n<h2>Actiepunten</h2>\n<ul>\n<li>☐ Jan — <strong>offerte</strong></li>\n<li>☑ klaar</li>\n</ul>\n<p>Tekst <em>schuin</em> en <code>code</code><br>regel twee</p>\n"
        );
    }

    #[test]
    fn numbered_lists() {
        assert_eq!(
            markdown_to_html("1. een\n2. twee\n- los"),
            "<ol>\n<li>een</li>\n<li>twee</li>\n</ol>\n<ul>\n<li>los</li>\n</ul>\n"
        );
        assert_eq!(
            markdown_to_html("2026. Een jaar"),
            "<ol>\n<li>Een jaar</li>\n</ol>\n"
        );
    }

    #[test]
    fn html_escapes() {
        assert_eq!(
            markdown_to_html("<script>a & b</script>"),
            "<p>&lt;script&gt;a &amp; b&lt;/script&gt;</p>\n"
        );
        assert_eq!(
            markdown_to_html("[x](javascript:alert(1))"),
            "<p>[x](javascript:alert(1))</p>\n"
        );
        assert_eq!(
            markdown_to_html("[site](https://a.be)"),
            "<p><a href=\"https://a.be\">site</a></p>\n"
        );
    }

    #[test]
    fn unbalanced_inline_marks_are_literal() {
        assert_eq!(
            markdown_to_html("**open en `half"),
            "<p>**open en `half</p>\n"
        );
        assert_eq!(
            markdown_to_html("**a** en **b"),
            "<p><strong>a</strong> en **b</p>\n"
        );
        assert_eq!(
            markdown_to_html("snake_case_naam"),
            "<p>snake_case_naam</p>\n"
        );
        assert_eq!(
            markdown_to_html("5 * 3 en *dit*"),
            "<p>5 * 3 en <em>dit</em></p>\n"
        );
        assert_eq!(
            markdown_to_html("`a*b` en *dit*"),
            "<p><code>a*b</code> en <em>dit</em></p>\n"
        );
        assert_eq!(
            markdown_to_html("***heel*** belangrijk"),
            "<p><strong><em>heel</em></strong> belangrijk</p>\n"
        );
        assert_eq!(
            markdown_to_html("[a*b](https://x.be/a*b)"),
            "<p><a href=\"https://x.be/a*b\">a*b</a></p>\n"
        );
    }
}
