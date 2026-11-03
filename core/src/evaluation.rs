//! Synthetic evaluation bridge. This module is absent from ordinary application builds.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::cleanup::{assess_cleanup, cleanup_messages, cleanup_timeout_secs, strip_wrapping};
use crate::llm::{ChatMessage, Reasoning};
use crate::meeting_chat::{chat_messages_with_tag, cited, select_sources};
use crate::meeting_index::{MeetingIndex, Passage, PassageKind};
use crate::meetings::{MeetingStore, Segment, Speaker, SpeakerName};
use crate::provider::{ProviderConfig, ProviderKind};
use crate::settings::Style;
use crate::summary::{BLOCK_CHARS, MAP_REDUCE_ABOVE, blocks, chunk_messages, summary_messages};
use crate::templates::TemplateStore;

pub const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, &'static str> {
        let path = std::env::temp_dir().join(format!("pennel-eval-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&path).map_err(|_| "fixture_io")?;
        Ok(Self(path))
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Deserialize)]
struct InputSegment {
    start_ms: u64,
    end_ms: u64,
    speaker: Speaker,
    text: String,
    #[serde(default)]
    failed: bool,
    #[serde(default)]
    voice: Option<u32>,
}

impl From<InputSegment> for Segment {
    fn from(input: InputSegment) -> Self {
        Self {
            start_ms: input.start_ms,
            end_ms: input.end_ms,
            speaker: input.speaker,
            text: input.text,
            failed: input.failed,
            voice: input.voice,
        }
    }
}

#[derive(Deserialize)]
struct InputName {
    voice: u32,
    name: String,
}

#[derive(Deserialize)]
struct InputMeeting {
    id: String,
    title: String,
    start_iso: String,
    #[serde(default, alias = "segments")]
    transcript: Vec<InputSegment>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    speaker_names: Vec<InputName>,
    #[serde(default = "general_template")]
    template_id: String,
}

fn general_template() -> String {
    "algemeen".into()
}

#[derive(Deserialize)]
struct InputQuery {
    id: String,
    question: String,
    #[serde(default)]
    meeting_id: Option<String>,
    #[serde(default)]
    history: Vec<crate::meeting_chat::ChatTurn>,
    #[serde(default)]
    apple: bool,
}

#[derive(Deserialize)]
struct CleanupPair {
    id: String,
    raw: String,
    cleaned: String,
    #[serde(default)]
    style: Option<String>,
    #[serde(default)]
    dictionary: Vec<String>,
}

#[derive(Deserialize)]
struct Suite {
    #[serde(default)]
    meetings: Vec<InputMeeting>,
    #[serde(default)]
    queries: Vec<InputQuery>,
    #[serde(default)]
    cleanup_pairs: Vec<CleanupPair>,
    #[serde(default)]
    citation_cases: Vec<Value>,
    #[serde(default)]
    include_timings: bool,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn style(value: Option<&str>) -> Result<Style, &'static str> {
    match value.unwrap_or("neutral") {
        "neutral" => Ok(Style::Neutral),
        "literal" => Ok(Style::Literal),
        "formal" => Ok(Style::Formal),
        "casual" => Ok(Style::Casual),
        _ => Err("invalid_style"),
    }
}

fn passage_value(passage: &Passage) -> Value {
    let signature = json!([
        passage.meeting_id,
        passage.kind,
        passage.start_ms,
        passage.text
    ]);
    let digest = crate::models::hex(&Sha256::digest(signature.to_string().as_bytes()));
    let mut result = serde_json::to_value(passage).expect("passage serialization");
    result["id"] = json!(format!("{}:{}", passage.meeting_id, &digest[..24]));
    result
}

fn passage_values(passages: &[Passage]) -> Vec<Value> {
    passages.iter().map(passage_value).collect()
}

fn source_tag(question: &str, sources: &[Passage]) -> String {
    let input = json!([question, sources]);
    let digest = crate::models::hex(&Sha256::digest(input.to_string().as_bytes()));
    format!("sources-{}", &digest[..32])
}

fn evaluate_suite(input: Value) -> Result<Value, &'static str> {
    let suite: Suite = serde_json::from_value(input).map_err(|_| "invalid_request")?;
    let mut ids = HashSet::new();
    for meeting in &suite.meetings {
        if !valid_id(&meeting.id)
            || !ids.insert(meeting.id.as_str())
            || meeting.title.contains(['\n', '\r'])
            || chrono::DateTime::parse_from_rfc3339(&meeting.start_iso).is_err()
            || meeting.transcript.iter().any(|s| s.end_ms < s.start_ms)
        {
            return Err("invalid_meeting");
        }
    }
    if suite
        .queries
        .iter()
        .any(|q| q.meeting_id.as_deref().is_some_and(|id| !ids.contains(id)))
    {
        return Err("unknown_meeting");
    }
    let root = OwnedRoot::new()?;
    let store = MeetingStore::new(&root.0.join("Meetings"));
    std::fs::create_dir(store.root()).map_err(|_| "fixture_io")?;
    let template_ids: HashMap<String, String> = suite
        .meetings
        .iter()
        .map(|meeting| (meeting.id.clone(), meeting.template_id.clone()))
        .collect();
    for meeting in suite.meetings {
        let directory = store.dir_of(&meeting.id);
        std::fs::create_dir(&directory).map_err(|_| "fixture_io")?;
        // Construct only the fixture's initial metadata; all transcript and note writes use the store.
        let metadata = format!(
            "---\ntitle: {}\nstart: {}\nduration_min: 0\ntemplate: {}\n---\n\n",
            meeting.title, meeting.start_iso, meeting.template_id
        );
        std::fs::write(directory.join("meeting.md"), metadata).map_err(|_| "fixture_io")?;
        let segments: Vec<Segment> = meeting.transcript.into_iter().map(Into::into).collect();
        store
            .write_transcript(&meeting.id, &segments)
            .map_err(|_| "fixture_io")?;
        let names = meeting
            .speaker_names
            .into_iter()
            .map(|n| SpeakerName {
                voice: n.voice,
                name: n.name,
            })
            .collect::<Vec<_>>();
        store
            .set_speaker_names(&meeting.id, &names)
            .map_err(|_| "fixture_io")?;
        store
            .write_notes(&meeting.id, &meeting.notes)
            .map_err(|_| "fixture_io")?;
        store
            .write_meeting(&meeting.id, 0, &meeting.template_id, &meeting.summary)
            .map_err(|_| "fixture_io")?;
    }
    let templates = TemplateStore::new(&store.root());
    let started = Instant::now();
    let mut index =
        MeetingIndex::open(&root.0.join("index.sqlite")).map_err(|_| "fixture_index")?;
    index.sync(&store).map_err(|_| "fixture_index")?;
    let index_elapsed = started.elapsed().as_nanos() as u64;
    let all = index
        .recent(&store, None, u32::MAX)
        .map_err(|_| "fixture_index")?;
    let mut queries = Vec::new();
    for query in suite.queries {
        let started = Instant::now();
        let (selected, mode) = select_sources(
            &index,
            &store,
            &query.question,
            &query.history,
            query.meeting_id.as_deref(),
            query.apple,
        )
        .map_err(|_| "fixture_query")?;
        let (messages, sources) = if query.apple {
            // Apple budgeting remains production code; only its random delimiter is canonicalized.
            let (mut messages, sources) = crate::meeting_chat::apple_chat_messages(
                &query.question,
                &query.history,
                &selected,
            )
            .map_err(|_| "unsupported_context")?;
            let old_tag = messages[0]
                .content
                .split("inside <")
                .nth(1)
                .and_then(|s| s.split('>').next())
                .ok_or("fixture_prompt")?
                .to_owned();
            let tag = source_tag(&query.question, &sources);
            for message in &mut messages {
                message.content = message.content.replace(&old_tag, &tag);
            }
            (messages, sources)
        } else {
            let messages = chat_messages_with_tag(
                &query.question,
                &query.history,
                &selected,
                &source_tag(&query.question, &selected),
            );
            (messages, selected)
        };
        let ask_elapsed = started.elapsed().as_nanos() as u64;
        let search_started = Instant::now();
        let search = index
            .search_grouped(&store, &query.question, 2)
            .map_err(|_| "fixture_query")?;
        let search_elapsed = search_started.elapsed().as_nanos() as u64;
        let mut result = json!({
            "id": query.id,
            "ask_mode": mode,
            "ask": passage_values(&sources),
            "search": passage_values(&search),
            "messages": messages,
        });
        if suite.include_timings {
            result["ask_elapsed_ns"] = json!(ask_elapsed);
            result["search_elapsed_ns"] = json!(search_elapsed);
        }
        queries.push(result);
    }
    let mut cleanup = Vec::new();
    for pair in suite.cleanup_pairs {
        let style = style(pair.style.as_deref())?;
        let started = Instant::now();
        let cleaned = strip_wrapping(&pair.cleaned);
        let accepted = assess_cleanup(&pair.raw, &cleaned, style, &pair.dictionary).accepted;
        let elapsed = started.elapsed().as_nanos() as u64;
        let mut result = json!({
            "id": pair.id,
            "accepted": accepted,
            "text": if accepted { &cleaned } else { &pair.raw },
            "messages": cleanup_messages(&pair.raw, style, &pair.dictionary),
            "timeout_secs": cleanup_timeout_secs(&pair.raw),
        });
        if suite.include_timings {
            result["elapsed_ns"] = json!(elapsed);
        }
        cleanup.push(result);
    }
    let mut summary_prompts = Vec::new();
    for meeting in store.list() {
        let segments = store.read_transcript(&meeting.id);
        let transcript =
            crate::summary::transcript_text(&segments, &store.speaker_names(&meeting.id));
        let notes = store.read_notes(&meeting.id);
        let template_id = template_ids.get(&meeting.id).ok_or("unknown_meeting")?;
        let spec = templates.spec(template_id);
        let map_reduce = transcript.chars().count() > MAP_REDUCE_ABOVE;
        let map_messages = if map_reduce {
            blocks(&transcript, BLOCK_CHARS)
                .iter()
                .map(|block| chunk_messages(block))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        summary_prompts.push(json!({
            "meeting_id": meeting.id,
            "map_reduce": map_reduce,
            "map_messages": map_messages,
            "messages": if map_reduce { Vec::new() } else { summary_messages(&spec, &notes, &transcript) },
            "notes": notes,
            "template_id": template_id,
        }));
    }
    let mut citation_cases = Vec::new();
    for case in suite.citation_cases {
        citation_cases.push(parse_citations(case)?);
    }
    let mut result = json!({
        "schema_version": 1,
        "passages": passage_values(&all),
        "queries": queries,
        "cleanup": cleanup,
        "templates": templates.list(),
        "summary_prompts": summary_prompts,
        "citation_cases": citation_cases,
    });
    if suite.include_timings {
        result["index_elapsed_ns"] = json!(index_elapsed);
    }
    Ok(result)
}

#[derive(Deserialize)]
struct InputPassage {
    meeting_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    start_iso: String,
    kind: String,
    #[serde(default)]
    start_ms: Option<u64>,
    text: String,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    meeting_uid: Option<String>,
    #[serde(default)]
    utterance_ids: Vec<String>,
    #[serde(default)]
    utterance_fingerprints: Vec<String>,
}

fn parse_citations(input: Value) -> Result<Value, &'static str> {
    let sources: Vec<InputPassage> =
        serde_json::from_value(input["sources"].clone()).map_err(|_| "invalid_request")?;
    let mut passages = Vec::new();
    for source in sources {
        passages.push(Passage {
            meeting_id: source.meeting_id,
            title: source.title,
            start_iso: source.start_iso,
            kind: match source.kind.as_str() {
                "notes" => PassageKind::Notes,
                "summary" => PassageKind::Summary,
                "transcript" => PassageKind::Transcript,
                _ => return Err("invalid_source_kind"),
            },
            start_ms: source.start_ms,
            text: source.text,
            snippet: source.snippet,
            meeting_uid: source.meeting_uid,
            utterance_ids: source.utterance_ids,
            utterance_fingerprints: source.utterance_fingerprints,
        });
    }
    let citations = cited(text(&input, "answer")?, &passages);
    Ok(json!({
        "id": input["id"],
        "numbers": citations.iter().map(|c| c.number).collect::<Vec<_>>(),
        "citations": citations.iter().map(|c| json!({"number":c.number,"passage":passage_value(&c.passage)})).collect::<Vec<_>>(),
    }))
}

fn text<'a>(input: &'a Value, field: &str) -> Result<&'a str, &'static str> {
    input[field].as_str().ok_or("invalid_request")
}

fn message_values(input: &Value) -> Result<Vec<ChatMessage>, &'static str> {
    input
        .as_array()
        .ok_or("invalid_request")?
        .iter()
        .map(|message| {
            let role = match text(message, "role")? {
                "system" => "system",
                "user" => "user",
                "assistant" => "assistant",
                _ => return Err("invalid_role"),
            };
            Ok(ChatMessage {
                role,
                content: text(message, "content")?.to_owned(),
            })
        })
        .collect()
}

/// Accepts fixture content only; no profile paths, credentials or network operations are supported.
pub fn evaluate(input: Value) -> Result<Value, &'static str> {
    match input["op"].as_str().unwrap_or("evaluate") {
        "evaluate" => evaluate_suite(input),
        "citations" => parse_citations(input),
        "cleanup_prompt" => {
            let dictionary = if input["dictionary"].is_null() {
                Vec::<String>::new()
            } else {
                serde_json::from_value(input["dictionary"].clone())
                    .map_err(|_| "invalid_request")?
            };
            Ok(
                json!({"messages":cleanup_messages(text(&input,"raw")?,style(input["style"].as_str())?,&dictionary)}),
            )
        }
        "cleanup_guard" => {
            let raw = text(&input, "raw")?;
            let cleaned = strip_wrapping(text(&input, "cleaned")?);
            let dictionary = if input["dictionary"].is_null() {
                Vec::<String>::new()
            } else {
                serde_json::from_value(input["dictionary"].clone())
                    .map_err(|_| "invalid_request")?
            };
            let style = style(input["style"].as_str())?;
            let accepted = assess_cleanup(raw, &cleaned, style, &dictionary).accepted;
            Ok(json!({"accepted":accepted,"text":if accepted { &cleaned } else { raw }}))
        }
        "summary_prompt" => {
            let root = OwnedRoot::new()?;
            let templates = TemplateStore::new(&root.0);
            let spec = templates.spec(input["template_id"].as_str().unwrap_or("algemeen"));
            Ok(
                json!({"messages":summary_messages(&spec,input["notes"].as_str().unwrap_or(""),text(&input,"transcript")?)}),
            )
        }
        "request_body" => {
            let provider = ProviderConfig {
                kind: match input["provider"].as_str().unwrap_or("custom") {
                    "open_router" | "openrouter" => ProviderKind::OpenRouter,
                    "ollama" => ProviderKind::Ollama,
                    "custom" => ProviderKind::Custom,
                    _ => return Err("unsupported_provider"),
                },
                model: text(&input, "model")?.into(),
                base_url: String::new(),
            };
            let reasoning = match input["reasoning"].as_str().unwrap_or("low") {
                "off" => Reasoning::Off,
                "low" => Reasoning::Low,
                _ => return Err("invalid_reasoning"),
            };
            Ok(
                json!({"body":crate::llm::request_body(&provider,&message_values(&input["messages"] )?,reasoning)}),
            )
        }
        _ => Err("unsupported_operation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        json!({
            "op":"evaluate",
            "meetings":[{"id":"fixture_en","title":"Synthetic budget","start_iso":"2026-01-01T10:00:00+00:00","transcript":[
                {"start_ms":0,"end_ms":1000,"speaker":"me","text":"The approved budget is fifty euros."},
                {"start_ms":1000,"end_ms":2000,"speaker":"others","voice":1,"text":"I am Zoë. I will review the budget."}
            ],"notes":"Remember the budget.","summary":"Budget approved."}],
            "queries":[{"id":"q1","question":"What is the budget?","meeting_id":"fixture_en"},{"id":"q2","question":"budget"},{"id":"q3","question":"de en het"}],
            "cleanup_pairs":[{"id":"negation","raw":"Stuur niet naar Jan","cleaned":"Stuur naar Jan"}]
        })
    }

    #[test]
    fn fixture_roundtrips_real_stores_and_is_reproducible() {
        let first = evaluate(fixture()).unwrap();
        let second = evaluate(fixture()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first["queries"][0]["ask_mode"], "whole_meeting");
        assert_eq!(first["queries"][1]["ask_mode"], "retrieved_library");
        assert_eq!(first["queries"][2]["ask_mode"], "recent_library");
        assert!(first["queries"][1]["ask"].as_array().unwrap().len() <= 12);
        assert!(first["passages"].as_array().unwrap().iter().all(|p| {
            p["meeting_id"] == "fixture_en" && p["id"].as_str().unwrap().starts_with("fixture_en:")
        }));
        assert_eq!(first["cleanup"][0]["accepted"], false);
        assert_eq!(first["cleanup"][0]["text"], "Stuur niet naar Jan");
        assert_eq!(first["templates"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn cleanup_bridge_uses_saved_style_and_dictionary() {
        for (raw, cleaned, settings, accepted) in [
            (
                "um send the checklist",
                "Send the checklist.",
                json!({}),
                true,
            ),
            ("open ai bouwt dit", "OpenAI bouwt dit.", json!({}), false),
            (
                "open ai bouwt dit",
                "OpenAI bouwt dit.",
                json!({"style":"neutral","dictionary":["OpenAI"]}),
                true,
            ),
            (
                "open ai bouwt dit",
                "OpenAI bouwt dit.",
                json!({"style":"literal","dictionary":["OpenAI"]}),
                false,
            ),
        ] {
            let mut pair = settings;
            pair["id"] = json!("cleanup");
            pair["raw"] = json!(raw);
            pair["cleaned"] = json!(cleaned);
            let mut input = fixture();
            input["cleanup_pairs"] = json!([pair.clone()]);
            let result = evaluate(input).unwrap();
            assert_eq!(result["cleanup"][0]["accepted"], accepted);
            assert_eq!(
                result["cleanup"][0]["text"],
                if accepted { cleaned } else { raw }
            );

            pair["op"] = json!("cleanup_guard");
            let guard = evaluate(pair).unwrap();
            assert_eq!(guard["accepted"], accepted);
            assert_eq!(guard["text"], if accepted { cleaned } else { raw });
        }
    }

    #[test]
    fn cleanup_guard_rejects_invalid_saved_settings() {
        let input = json!({"op":"cleanup_guard","raw":"send it","cleaned":"Send it."});
        let mut invalid = input.clone();
        invalid["style"] = json!("unknown");
        assert_eq!(evaluate(invalid), Err("invalid_style"));
        let mut invalid = input;
        invalid["dictionary"] = json!([17]);
        assert_eq!(evaluate(invalid), Err("invalid_request"));
    }

    #[test]
    fn path_injection_and_invalid_scopes_are_rejected() {
        let mut input = fixture();
        input["meetings"][0]["id"] = json!("../outside");
        assert_eq!(evaluate(input), Err("invalid_meeting"));
        let mut input = fixture();
        input["queries"][0]["meeting_id"] = json!("unknown");
        assert_eq!(evaluate(input), Err("unknown_meeting"));
    }

    #[test]
    fn citation_parsing_is_syntax_only_and_ignores_unknown_ids() {
        let input = json!({"op":"citations","answer":"Unsupported statement [1, 99].","sources":[{"meeting_id":"fixture","kind":"transcript","text":"Different fact."}]});
        let result = evaluate(input).unwrap();
        assert_eq!(result["numbers"], json!([1]));
        assert_eq!(result["citations"][0]["passage"]["text"], "Different fact.");
    }

    #[test]
    fn long_meeting_uses_actual_retrieval_and_map_reduce_boundaries() {
        let mut input = fixture();
        input["meetings"][0]["transcript"][0]["text"] = json!("synthetic distractor ".repeat(4000));
        let result = evaluate(input).unwrap();
        assert_eq!(result["queries"][0]["ask_mode"], "retrieved_meeting");
        assert_eq!(result["summary_prompts"][0]["map_reduce"], true);
        assert!(
            result["summary_prompts"][0]["messages"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            !result["summary_prompts"][0]["map_messages"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn generative_request_body_uses_production_reasoning_settings() {
        let result = evaluate(json!({"op":"request_body","provider":"openrouter","model":"fixture","reasoning":"off","messages":[{"role":"user","content":"synthetic"}]})).unwrap();
        assert_eq!(result["body"]["temperature"], 0);
        assert_eq!(result["body"]["reasoning"]["enabled"], false);
    }
}
