//! Claude access: answers the read-only meeting tools Claude calls through Pennel Cloud's relay,
//! from this device's own meetings. Arguments and answers are never logged.

use serde_json::Value;

use crate::cloud::relay::{RelayAnswer, RelayClient, RelayRequest};
use crate::engine::{ApiKeys, PennelEngine};
use crate::error::PennelError;
use crate::meeting_chat::search_terms;
use crate::meeting_index::Passage;
use crate::meetings::{MeetingInfo, speaker_label, timestamp};

pub const MAX_ANSWER_CHARS: usize = 200_000;
const PASSAGE_CHARS: usize = 2_000;
const FALLBACK_PASSAGES: u32 = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Summary,
    Notes,
    Transcript,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ToolCall {
    Search {
        query: String,
        limit: usize,
    },
    Recent {
        limit: usize,
    },
    Get {
        meeting_id: String,
        part: Part,
    },
    Ask {
        question: String,
        meeting_id: Option<String>,
    },
}

fn invalid(message: &str) -> RelayAnswer {
    RelayAnswer::Failed {
        code: "invalid_arguments".into(),
        message: message.into(),
    }
}

fn not_found() -> RelayAnswer {
    RelayAnswer::Failed {
        code: "not_found".into(),
        message: "This meeting no longer exists on this computer.".into(),
    }
}

fn text_arg(
    args: &Value,
    name: &str,
    max_chars: usize,
    message: &str,
) -> Result<Option<String>, RelayAnswer> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            let text = text.trim();
            if text.is_empty() || text.chars().count() > max_chars {
                Err(invalid(message))
            } else {
                Ok(Some(text.to_owned()))
            }
        }
        Some(_) => Err(invalid(message)),
    }
}

fn limit_arg(args: &Value, max: u64, default: usize) -> Result<usize, RelayAnswer> {
    match args.get("limit") {
        None | Some(Value::Null) => Ok(default),
        Some(value) => match value.as_u64() {
            Some(limit) if (1..=max).contains(&limit) => Ok(limit as usize),
            _ => Err(invalid(&format!("`limit` must be between 1 and {max}."))),
        },
    }
}

fn meeting_id_arg(args: &Value, required: bool) -> Result<Option<String>, RelayAnswer> {
    let id = text_arg(args, "meeting_id", 200, "`meeting_id` is invalid.")?;
    match id {
        Some(id) if id.contains(['/', '\\']) || id.contains("..") => Err(not_found()),
        None if required => Err(invalid("`meeting_id` is required.")),
        other => Ok(other),
    }
}

fn parse_call(tool: &str, args: &Value) -> Result<ToolCall, RelayAnswer> {
    if !args.is_object() {
        return Err(invalid("The arguments must be an object."));
    }
    match tool {
        "search_meetings" => Ok(ToolCall::Search {
            query: text_arg(
                args,
                "query",
                200,
                "`query` must be between 1 and 200 characters long.",
            )?
            .ok_or_else(|| invalid("`query` is required."))?,
            limit: limit_arg(args, 20, 10)?,
        }),
        "list_recent_meetings" => Ok(ToolCall::Recent {
            limit: limit_arg(args, 50, 10)?,
        }),
        "get_meeting" => {
            let meeting_id = meeting_id_arg(args, true)?.unwrap_or_default();
            let part = match args.get("part").and_then(Value::as_str) {
                None if args.get("part").is_none_or(Value::is_null) => Part::All,
                Some("all") => Part::All,
                Some("summary") => Part::Summary,
                Some("notes") => Part::Notes,
                Some("transcript") => Part::Transcript,
                _ => {
                    return Err(invalid("`part` must be summary, notes, transcript or all."));
                }
            };
            Ok(ToolCall::Get { meeting_id, part })
        }
        "ask_meetings" => Ok(ToolCall::Ask {
            question: text_arg(
                args,
                "question",
                1000,
                "`question` must be between 1 and 1000 characters long.",
            )?
            .ok_or_else(|| invalid("`question` is required."))?,
            meeting_id: meeting_id_arg(args, false)?,
        }),
        _ => Err(invalid("Pennel does not recognize this tool.")),
    }
}

/// `2026-10-20T09:00:00+02:00` → `2026-10-20 09:00`.
fn when(start_iso: &str) -> String {
    start_iso
        .get(..16)
        .unwrap_or(start_iso)
        .replacen('T', " ", 1)
}

fn meeting_line(info: &MeetingInfo) -> String {
    let summary = if info.has_summary {
        ", with summary"
    } else {
        ""
    };
    format!(
        "- **{}** — {}, {} min{summary} (meeting_id: `{}`)",
        info.title,
        when(&info.start_iso),
        info.duration_min,
        info.id
    )
}

fn clip(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

/// Cuts at `MAX_ANSWER_CHARS` and says so, so Claude knows to ask for a smaller part.
pub fn truncate_answer(text: String) -> String {
    match text.char_indices().nth(MAX_ANSWER_CHARS) {
        Some((cut, _)) => format!(
            "{}\n\n[Truncated: this answer exceeded {MAX_ANSWER_CHARS} characters. Request a smaller part.]",
            &text[..cut]
        ),
        None => text,
    }
}

fn passage_block(number: usize, passage: &Passage) -> String {
    format!(
        "[{number}] **{}** ({}, meeting_id: `{}`)\n{}",
        passage.title,
        when(&passage.start_iso),
        passage.meeting_id,
        clip(passage.text.trim(), PASSAGE_CHARS)
    )
}

impl PennelEngine {
    fn known_meeting(&self, id: &str) -> Option<MeetingInfo> {
        self.meetings().into_iter().find(|info| info.id == id)
    }

    fn answer_search(&self, query: &str, limit: usize) -> String {
        let found = self.search_library(query.to_owned());
        if found.meetings.is_empty() {
            return format!("No meetings found for «{query}».");
        }
        let mut out = format!("Meetings for «{query}»:\n");
        for info in found.meetings.iter().take(limit) {
            out.push('\n');
            out.push_str(&meeting_line(info));
            for passage in found
                .passages
                .iter()
                .filter(|p| p.meeting_id == info.id)
                .take(2)
            {
                out.push_str(&format!("\n  > {}", passage.snippet.trim()));
            }
        }
        out
    }

    fn answer_recent(&self, limit: usize) -> String {
        let meetings = self.meetings();
        if meetings.is_empty() {
            return "There are no meetings on this computer yet.".into();
        }
        let lines: Vec<String> = meetings.iter().take(limit).map(meeting_line).collect();
        format!("Most recent meetings:\n\n{}", lines.join("\n"))
    }

    fn transcript_text(&self, id: &str) -> String {
        let names = self.meeting_speakers(id.to_owned());
        self.meeting_transcript(id.to_owned())
            .into_iter()
            .filter(|s| !s.failed && !s.text.trim().is_empty())
            .map(|s| {
                format!(
                    "[{}] {}: {}",
                    timestamp(s.start_ms),
                    speaker_label(&s, &names),
                    s.text.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn answer_get(&self, id: &str, part: &Part) -> Result<String, RelayAnswer> {
        let info = self.known_meeting(id).ok_or_else(not_found)?;
        let summary = self.meeting_markdown(id.to_owned()).unwrap_or_default();
        let notes = self.meeting_notes(id.to_owned());
        let section = |name: &str, body: &str, empty: &str| {
            let body = body.trim();
            format!(
                "## {name}\n\n{}",
                if body.is_empty() { empty } else { body }
            )
        };
        let summary_part = || section("Summary", &summary, "No summary yet.");
        let notes_part = || section("Notes", &notes, "No notes.");
        let transcript_part = || section("Transcript", &self.transcript_text(id), "No transcript.");
        let body = match part {
            Part::Summary => summary_part(),
            Part::Notes => notes_part(),
            Part::Transcript => transcript_part(),
            Part::All => [summary_part(), notes_part(), transcript_part()].join("\n\n"),
        };
        Ok(format!(
            "# {}\n\n{}, {} min (meeting_id: `{}`)\n\n{body}",
            info.title,
            when(&info.start_iso),
            info.duration_min,
            info.id
        ))
    }

    fn relevant_passages(&self, question: &str, scope: Option<&str>) -> Vec<Passage> {
        let store = self.meeting_store();
        let terms = search_terms(question);
        self.with_index(&store, |index| {
            let found = index.search_any(&store, &terms, scope, FALLBACK_PASSAGES)?;
            if found.is_empty() {
                index.recent(&store, scope, FALLBACK_PASSAGES)
            } else {
                Ok(found)
            }
        })
        .unwrap_or_default()
    }

    fn answer_ask(
        &self,
        question: &str,
        meeting_id: Option<&str>,
        keys: ApiKeys,
    ) -> Result<String, RelayAnswer> {
        if let Some(id) = meeting_id {
            self.known_meeting(id).ok_or_else(not_found)?;
        }
        match self.ask_meetings(
            question.to_owned(),
            Vec::new(),
            meeting_id.map(str::to_owned),
            keys,
        ) {
            Ok(answer) => {
                let mut out = answer.text.trim().to_owned();
                if !answer.citations.is_empty() {
                    out.push_str("\n\nSources:");
                    for citation in &answer.citations {
                        let p = &citation.passage;
                        out.push_str(&format!(
                            "\n[{}] {} ({}, meeting_id: `{}`)",
                            citation.number,
                            p.title,
                            when(&p.start_iso),
                            p.meeting_id
                        ));
                    }
                }
                Ok(out)
            }
            // Without a working model on this computer, Claude answers from the passages itself.
            Err(_) => {
                let passages = self.relevant_passages(question, meeting_id);
                if passages.is_empty() {
                    return Ok("There are no meetings about this on this computer.".into());
                }
                let blocks: Vec<String> = passages
                    .iter()
                    .enumerate()
                    .map(|(index, passage)| passage_block(index + 1, passage))
                    .collect();
                Ok(format!(
                    "Pennel could not generate an answer on this computer (no language model available). These are the most relevant meeting passages; use them to answer the question:\n\n{}",
                    blocks.join("\n\n")
                ))
            }
        }
    }
}

impl PennelEngine {
    /// Desktop-only capability negotiation; the mobile relay contract stays unchanged.
    pub fn cloud_relay_poll_with_capabilities(
        &self,
        client: &str,
        capabilities: &[String],
    ) -> Result<Vec<RelayRequest>, PennelError> {
        if !self.cloud_signed_in() {
            return Err(PennelError::NotSignedIn);
        }
        RelayClient::new(self.cloud_session()?).poll_with_capabilities(client, capabilities)
    }

    pub fn cloud_writing_style_source(&self, enabled: Option<bool>) -> Result<bool, PennelError> {
        if !self.cloud_signed_in() {
            return Err(PennelError::NotSignedIn);
        }
        RelayClient::new(self.cloud_session()?).writing_style_source(enabled)
    }

    pub fn cloud_writing_style_mcp_url(&self) -> String {
        format!("{}/mcp/style", self.cloud_base_url())
    }

    /// Identity changes clear permission while holding this same gate. A late designation must
    /// not restore the old account's permission just before the new credentials are installed.
    pub fn enable_writing_profile_sharing_for_account(
        &self,
        expected_epoch: u64,
    ) -> Result<(), PennelError> {
        let _identity = crate::speaker_jobs::speaker_identity_guard();
        if !self.cloud_signed_in() || self.cloud_account_epoch() != expected_epoch {
            return Err(PennelError::Cancelled);
        }
        self.set_writing_profile_sharing(true)?;
        Ok(())
    }
}

#[uniffi::export]
impl PennelEngine {
    /// Answers one relayed tool call from this device's meetings. Read-only.
    pub fn claude_access_answer(
        &self,
        tool: String,
        arguments_json: String,
        keys: ApiKeys,
    ) -> RelayAnswer {
        let args: Value = match serde_json::from_str(&arguments_json) {
            Ok(args) => args,
            Err(_) => return invalid("The arguments are not valid JSON."),
        };
        let call = match parse_call(&tool, &args) {
            Ok(call) => call,
            Err(answer) => return answer,
        };
        let result = match &call {
            ToolCall::Search { query, limit } => Ok(self.answer_search(query, *limit)),
            ToolCall::Recent { limit } => Ok(self.answer_recent(*limit)),
            ToolCall::Get { meeting_id, part } => self.answer_get(meeting_id, part),
            ToolCall::Ask {
                question,
                meeting_id,
            } => self.answer_ask(question, meeting_id.as_deref(), keys),
        };
        match result {
            Ok(text) => RelayAnswer::Text {
                text: truncate_answer(text),
            },
            Err(answer) => answer,
        }
    }

    pub fn cloud_relay_poll(&self, client: String) -> Result<Vec<RelayRequest>, PennelError> {
        if !self.cloud_signed_in() {
            return Err(PennelError::NotSignedIn);
        }
        RelayClient::new(self.cloud_session()?).poll(&client)
    }

    pub fn cloud_relay_respond(&self, id: String, answer: RelayAnswer) -> Result<(), PennelError> {
        RelayClient::new(self.cloud_session()?).respond(&id, &answer)
    }

    pub fn cloud_mcp_url(&self) -> String {
        format!("{}/mcp", self.cloud_base_url())
    }

    /// Switching on needs a signed-in account; switching off always works.
    pub fn set_claude_access(&self, enabled: bool) -> Result<(), PennelError> {
        if enabled && !self.cloud_signed_in() {
            return Err(PennelError::NotSignedIn);
        }
        self.update_settings_with(|mut settings| {
            settings.claude_access = enabled;
            Ok(settings)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::{Segment, Speaker};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn engine(dir: &tempfile::TempDir, llm_url: &str) -> Arc<PennelEngine> {
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.providers[0].base_url = llm_url.into();
        settings.meetings_dir = dir.path().join("meetings").to_string_lossy().into_owned();
        engine.update_settings(settings).unwrap();
        std::fs::create_dir_all(engine.settings().meetings_dir).unwrap();
        engine
    }

    #[test]
    fn profile_designation_cannot_enable_permission_for_a_replaced_identity() {
        use crate::cloud::tests::MemorySecrets;
        use crate::cloud::{REFRESH_ACCOUNT, SecretStore};
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into()).unwrap();
        let first = Arc::new(MemorySecrets::default());
        first.set(
            REFRESH_ACCOUNT.into(),
            Some("synthetic-first-session".into()),
        );
        engine.set_secret_store(first).unwrap();
        assert!(engine.cloud_signed_in());
        let old = engine.cloud_account_epoch();
        let second = Arc::new(MemorySecrets::default());
        second.set(
            REFRESH_ACCOUNT.into(),
            Some("synthetic-second-session".into()),
        );
        engine.set_secret_store(second).unwrap();
        assert!(engine.cloud_signed_in());
        assert!(
            engine
                .enable_writing_profile_sharing_for_account(old)
                .is_err()
        );
        assert!(!engine.writing_profile_sharing_enabled());
        engine
            .enable_writing_profile_sharing_for_account(engine.cloud_account_epoch())
            .unwrap();
        assert!(engine.writing_profile_sharing_enabled());
        // Replacing credentials clears sharing without any network logout against a real host.
        engine
            .set_secret_store(Arc::new(MemorySecrets::default()))
            .unwrap();
        assert!(!engine.writing_profile_sharing_enabled());
    }

    fn add(engine: &PennelEngine, title: &str, hour: u32, said: &str) -> String {
        use chrono::TimeZone;
        let store = engine.meeting_store();
        let id = store
            .create(
                title,
                chrono::Local
                    .with_ymd_and_hms(2026, 10, 20, hour, 0, 0)
                    .unwrap(),
            )
            .unwrap();
        let segment = Segment {
            start_ms: 61_000,
            end_ms: 62_000,
            speaker: Speaker::Others,
            text: said.into(),
            failed: false,
            voice: None,
        };
        store.write_transcript(&id, &[segment]).unwrap();
        id
    }

    fn text(answer: RelayAnswer) -> String {
        match answer {
            RelayAnswer::Text { text } => text,
            other => panic!("expected text, got {other:?}"),
        }
    }

    fn code(answer: RelayAnswer) -> String {
        match answer {
            RelayAnswer::Failed { code, .. } => code,
            other => panic!("expected failure, got {other:?}"),
        }
    }

    fn no_keys() -> ApiKeys {
        ApiKeys {
            by_provider: HashMap::new(),
        }
    }

    fn call(engine: &PennelEngine, tool: &str, args: serde_json::Value) -> RelayAnswer {
        engine.claude_access_answer(tool.into(), args.to_string(), no_keys())
    }

    #[test]
    fn rejects_bad_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir, "http://127.0.0.1:9");
        let cases = [
            ("search_meetings", serde_json::json!({})),
            ("search_meetings", serde_json::json!({"query": "  "})),
            (
                "search_meetings",
                serde_json::json!({"query": "x".repeat(201)}),
            ),
            (
                "search_meetings",
                serde_json::json!({"query": "a", "limit": 21}),
            ),
            ("list_recent_meetings", serde_json::json!({"limit": 0})),
            ("list_recent_meetings", serde_json::json!({"limit": "5"})),
            ("get_meeting", serde_json::json!({})),
            (
                "get_meeting",
                serde_json::json!({"meeting_id": "a", "part": "audio"}),
            ),
            (
                "ask_meetings",
                serde_json::json!({"question": "q".repeat(1001)}),
            ),
            ("delete_meeting", serde_json::json!({})),
        ];
        for (tool, args) in cases {
            assert_eq!(
                code(call(&engine, tool, args.clone())),
                "invalid_arguments",
                "{tool} {args}"
            );
        }
        assert_eq!(
            code(engine.claude_access_answer("search_meetings".into(), "nope".into(), no_keys())),
            "invalid_arguments"
        );
    }

    #[test]
    fn tool_validation_uses_english_without_changing_error_codes() {
        for (tool, args, message) in [
            (
                "search_meetings",
                serde_json::json!({}),
                "`query` is required.",
            ),
            (
                "get_meeting",
                serde_json::json!({}),
                "`meeting_id` is required.",
            ),
            (
                "list_recent_meetings",
                serde_json::json!({"limit": 0}),
                "`limit` must be between 1 and 50.",
            ),
        ] {
            assert_eq!(
                parse_call(tool, &args),
                Err(RelayAnswer::Failed {
                    code: "invalid_arguments".into(),
                    message: message.into(),
                })
            );
        }
    }

    #[test]
    fn search_lists_matching_meetings_with_snippets() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir, "http://127.0.0.1:9");
        let budget = add(&engine, "Budget review", 9, "het budget is goedgekeurd");
        add(&engine, "Lunch", 10, "pasta");
        let answer = text(call(
            &engine,
            "search_meetings",
            serde_json::json!({"query": "budget"}),
        ));
        assert!(answer.contains("**Budget review**"), "{answer}");
        assert!(
            answer.contains(&format!("meeting_id: `{budget}`")),
            "{answer}"
        );
        assert!(answer.contains("goedgekeurd"), "{answer}");
        assert!(!answer.contains("Lunch"), "{answer}");
        let none = text(call(
            &engine,
            "search_meetings",
            serde_json::json!({"query": "kiwi"}),
        ));
        assert_eq!(none, "No meetings found for «kiwi».");
    }

    #[test]
    fn recent_lists_newest_first_up_to_limit() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir, "http://127.0.0.1:9");
        assert_eq!(
            text(call(&engine, "list_recent_meetings", serde_json::json!({}))),
            "There are no meetings on this computer yet."
        );
        add(&engine, "Oud", 8, "a");
        add(&engine, "Nieuw", 11, "b");
        let answer = text(call(
            &engine,
            "list_recent_meetings",
            serde_json::json!({"limit": 1}),
        ));
        assert!(answer.contains("**Nieuw** — 2026-10-20 11:00"), "{answer}");
        assert!(!answer.contains("Oud"), "{answer}");
    }

    #[test]
    fn get_meeting_returns_each_part() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir, "http://127.0.0.1:9");
        let id = add(&engine, "Sync", 9, "we verhuizen naar Gent");
        engine
            .meeting_store()
            .write_meeting(&id, 30, "algemeen", "## Besluiten\n- Gent")
            .unwrap();
        engine.meeting_store().write_notes(&id, "bel Jan").unwrap();
        let get = |part: &str| {
            text(call(
                &engine,
                "get_meeting",
                serde_json::json!({"meeting_id": id, "part": part}),
            ))
        };
        let summary = get("summary");
        assert!(
            summary.starts_with("# Sync\n\n2026-10-20 09:00, 30 min"),
            "{summary}"
        );
        assert!(summary.contains("## Summary\n"), "{summary}");
        assert!(summary.contains("## Besluiten\n- Gent") && !summary.contains("bel Jan"));
        assert!(get("notes").contains("## Notes\n\nbel Jan"));
        assert!(get("transcript").contains("[01:01] Anderen: we verhuizen naar Gent"));
        let all = text(call(
            &engine,
            "get_meeting",
            serde_json::json!({"meeting_id": id}),
        ));
        assert!(all.contains("Gent") && all.contains("bel Jan") && all.contains("[01:01]"));
    }

    #[test]
    fn get_meeting_refuses_unknown_and_escaping_ids() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir, "http://127.0.0.1:9");
        add(&engine, "Sync", 9, "x");
        for id in ["bestaat-niet", "../meetings", "a/b"] {
            assert_eq!(
                code(call(
                    &engine,
                    "get_meeting",
                    serde_json::json!({"meeting_id": id})
                )),
                "not_found",
                "{id}"
            );
        }
    }

    #[test]
    fn ask_uses_the_configured_model_and_lists_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine(&dir, &server.url());
        add(&engine, "Sync", 9, "we verhuizen naar Gent");
        let _llm = server
            .mock("POST", "/chat/completions")
            .with_body(r#"{"choices":[{"message":{"content":"Naar Gent [1]."}}]}"#)
            .create();
        let keys = ApiKeys {
            by_provider: HashMap::from([("groq".to_owned(), "k".to_owned())]),
        };
        let answer = text(engine.claude_access_answer(
            "ask_meetings".into(),
            serde_json::json!({"question": "Waar verhuizen we naartoe?"}).to_string(),
            keys,
        ));
        assert!(answer.starts_with("Naar Gent [1]."), "{answer}");
        assert!(
            answer.contains("Sources:\n[1] Sync (2026-10-20 09:00"),
            "{answer}"
        );
    }

    #[test]
    fn ask_without_a_model_returns_relevant_passages() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir, "http://127.0.0.1:9");
        add(&engine, "Sync", 9, "we verhuizen naar Gent");
        add(&engine, "Lunch", 10, "pasta");
        let answer = text(call(
            &engine,
            "ask_meetings",
            serde_json::json!({"question": "Waar verhuizen we naartoe?"}),
        ));
        assert!(
            answer.starts_with("Pennel could not generate an answer on this computer"),
            "{answer}"
        );
        assert!(
            answer.contains("[1] **Sync**") && answer.contains("verhuizen naar Gent"),
            "{answer}"
        );
        assert!(!answer.contains("pasta"), "{answer}");
    }

    #[test]
    fn long_answers_are_cut_with_a_note() {
        let long = "é".repeat(MAX_ANSWER_CHARS + 10);
        let cut = truncate_answer(long);
        assert!(cut.starts_with(&"é".repeat(MAX_ANSWER_CHARS)));
        assert!(cut.ends_with("Request a smaller part.]"));
        assert_eq!(truncate_answer("kort".into()), "kort");
    }

    #[test]
    fn claude_access_needs_a_signed_in_account_to_switch_on() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir, "http://127.0.0.1:9");
        assert!(!engine.settings().claude_access);
        assert_eq!(
            engine.set_claude_access(true),
            Err(PennelError::NotSignedIn)
        );
        assert_eq!(engine.set_claude_access(false), Ok(()));
    }

    #[test]
    fn signing_out_switches_claude_access_off() {
        use crate::cloud::account::Session;
        use crate::cloud::tests::MemorySecrets;
        use crate::cloud::{REFRESH_ACCOUNT, SecretStore};
        let dir = tempfile::tempdir().unwrap();
        let server = mockito::Server::new();
        let engine = engine(&dir, "http://127.0.0.1:9");
        let secrets = Arc::new(MemorySecrets::default());
        secrets.set(REFRESH_ACCOUNT.into(), Some("ref".into()));
        engine.set_secret_store(secrets.clone()).unwrap();
        engine.use_cloud_session(Session::new(server.url(), secrets));
        engine.set_claude_access(true).unwrap();
        assert!(engine.settings().claude_access);
        engine.cloud_logout().unwrap();
        assert!(!engine.settings().claude_access);
    }
}
