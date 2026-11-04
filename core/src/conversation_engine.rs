use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::conversations::*;
use crate::{
    ApiKeys, InferenceCancellation, MeetingAnswer, MeetingEvidence, MeetingQuestionScope,
    MeetingQuestionSelection, MeetingSourceAnchor, Passage, PassageKind, PennelEngine, PennelError,
};
use serde::de::DeserializeOwned;

#[derive(Clone)]
pub(crate) struct ActiveConversationRequest {
    conversation_id: String,
    scope_key: String,
    library: String,
    scope: Scope,
    assistant_id: String,
    cancellation: Arc<InferenceCancellation>,
    session: Option<Weak<crate::MeetingSession>>,
    session_id: Option<String>,
    epoch: u64,
}
struct FrozenContext {
    base: MeetingEvidence,
    evidence: Evidence,
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T, PennelError> {
    if text.len() > 512 * 1024 {
        return Err(invalid("The conversation request is too large."));
    }
    serde_json::from_str(text).map_err(|_| invalid("The conversation request is invalid."))
}
fn output(value: &impl serde::Serialize) -> Result<String, PennelError> {
    serde_json::to_string(value).map_err(PennelError::storage)
}
fn title(value: Option<String>) -> Result<String, PennelError> {
    let value = value.unwrap_or_default();
    if value.chars().count() > 256 || value.chars().any(char::is_control) {
        return Err(invalid("Enter a single-line conversation title."));
    }
    Ok(value.trim().into())
}
fn conversation_read_recovery_reason(error: &PennelError) -> Option<String> {
    match error {
        PennelError::InvalidResponse { message }
            if [
                "The saved conversation metadata could not be read. Its original data was kept.",
                "A saved conversation message could not be read. Its original data was kept.",
                "The saved conversation scope could not be read.",
            ]
            .contains(&message.as_str()) =>
        {
            Some(message.clone())
        }
        PennelError::Storage { .. } => Some(STORAGE_ERROR.into()),
        _ => None,
    }
}
fn library(engine: &PennelEngine) -> Result<String, PennelError> {
    crate::storage::private_library_key(&engine.meeting_store().root())
}

fn conversation_message_page(
    doc: &Cached,
    store: Option<&Store>,
    request: &ReadRequest,
) -> Result<(Vec<Message>, Option<String>), PennelError> {
    let store = store.filter(|_| doc.persisted);
    let upper = if let Some(focus) = request.focus_message_id.as_ref() {
        doc.messages
            .iter()
            .find(|message| &message.id == focus)
            .map(|message| message.position)
            .or(store
                .map(|store| store.message(&doc.summary.id, focus))
                .transpose()?
                .flatten()
                .map(|message| message.position))
            .ok_or(PennelError::NotFound)?
            + 1
    } else {
        request
            .messages_before
            .as_deref()
            .map(|cursor| {
                cursor
                    .parse::<i64>()
                    .map_err(|_| invalid("The conversation page cursor is invalid."))
            })
            .transpose()?
            .unwrap_or(i64::MAX)
    };
    let cursor = (upper != i64::MAX).then(|| upper.to_string());
    let (saved, older) = store
        .map(|store| store.page(&doc.summary.id, cursor.as_deref(), None))
        .transpose()?
        .unwrap_or_default();
    let mut merged: HashMap<String, Message> = saved
        .into_iter()
        .map(|message| (message.id.clone(), message))
        .collect();
    for message in doc
        .messages
        .iter()
        .filter(|message| message.position < upper)
    {
        merged.insert(message.id.clone(), message.clone());
    }
    let mut ordered: Vec<_> = merged.into_values().collect();
    ordered.sort_by_key(|message| std::cmp::Reverse(message.position));
    let mut page = vec![];
    let mut bytes = 0;
    for message in &ordered {
        let size = serde_json::to_vec(message)
            .map_err(PennelError::storage)?
            .len();
        if page.len() >= PAGE || (!page.is_empty() && bytes + size > 4 * 1024 * 1024) {
            break;
        }
        bytes += size;
        page.push(message.clone());
    }
    let next = (ordered.len() > page.len() || older.is_some())
        .then(|| page.last().map(|message| message.position.to_string()))
        .flatten();
    page.reverse();
    Ok((page, next))
}

impl PennelEngine {
    fn normalize_conversation_scope(&self, scope: Scope) -> Result<(Scope, String), PennelError> {
        self.normalize_conversation_scope_retry(scope, false)
    }
    fn normalize_conversation_scope_retry(
        &self,
        scope: Scope,
        retry: bool,
    ) -> Result<(Scope, String), PennelError> {
        self.claim_library()?;
        let _sync = self.sync_gate.lock().unwrap();
        let library = library(self)?;
        let Scope::Meeting {
            meeting_uid: requested,
            meeting_id: hint,
        } = scope
        else {
            return Ok((Scope::Library, library));
        };
        if hint.as_deref().is_some_and(|hint| {
            std::path::Path::new(hint).components().count() != 1 || hint.contains(['/', '\\'])
        }) {
            return Err(PennelError::NotFound);
        }
        // The alias is proof of an identity write made by this engine, never a guessed name/content match.
        let mut state = self.conversations.lock().unwrap();
        let alias = state
            .legacy_aliases
            .get(&(library.clone(), requested.clone()))
            .cloned();
        let resolved = alias.as_deref().unwrap_or(&requested);
        let store = self.meeting_store();
        let id = if let Some(current) = self.meeting_id_for_uid(resolved) {
            if hint.as_deref().is_some_and(|hint| {
                store.info(hint).is_some() && store.uid_of(hint).as_deref() != Some(resolved)
            }) {
                return Err(invalid("This conversation belongs to another meeting."));
            }
            current
        } else if alias.is_some() {
            return Err(PennelError::NotFound);
        } else if let Some(hint) = hint {
            if store.read_transcript_snapshot(&hint)?.meeting_uid != requested {
                return Err(invalid("This conversation belongs to another meeting."));
            }
            hint
        } else {
            store
                .list()
                .into_iter()
                .find_map(|meeting| {
                    store
                        .read_transcript_snapshot(&meeting.id)
                        .ok()
                        .filter(|snapshot| snapshot.meeting_uid == requested)
                        .map(|_| meeting.id)
                })
                .ok_or(PennelError::NotFound)?
        };
        let temporary = Scope::Meeting {
            meeting_uid: requested.clone(),
            meeting_id: Some(id.clone()),
        };
        let old_key = temporary.key(&library);
        let has_temporary = state
            .cache
            .values()
            .any(|doc| doc.scope_key == old_key && requested.starts_with("legacy-"));
        let active = state.active.values().any(|job| job.scope_key == old_key);
        if requested.starts_with("legacy-")
            && alias.is_none()
            && has_temporary
            && (!retry || active)
        {
            return Ok((temporary, library));
        }
        let stable = if let Some(stable) = store.uid_of(&id) {
            stable
        } else {
            let _guard = self.people_metadata_guard(&id)?;
            match store.ensure_uid(&id) {
                Ok(stable) => stable,
                Err(_) if requested.starts_with("legacy-") => return Ok((temporary, library)),
                Err(error) => return Err(error),
            }
        };
        let canonical = Scope::Meeting {
            meeting_uid: stable.clone(),
            meeting_id: None,
        };
        if requested != stable && requested.starts_with("legacy-") {
            state
                .legacy_aliases
                .insert((library.clone(), requested), stable);
            self.promote_temporary_conversations(&mut state, &old_key, &canonical, &library);
        }
        Ok((canonical, library))
    }
    fn promote_temporary_conversations(
        &self,
        state: &mut State,
        old_key: &str,
        scope: &Scope,
        library: &str,
    ) {
        let new_key = scope.key(library);
        let active: Vec<_> = state
            .active
            .iter()
            .filter(|(_, job)| job.scope_key == old_key)
            .map(|(id, _)| id.clone())
            .collect();
        for id in active {
            if let Some(job) = state.active.remove(&id) {
                job.cancellation.cancel();
                self.finish_inference(&job.cancellation);
                if let Some(doc) = state.cache.get_mut(&job.conversation_id) {
                    doc.summary.active_request_id = None;
                    if let Some(message) = doc
                        .messages
                        .iter_mut()
                        .find(|message| message.id == job.assistant_id)
                    {
                        message.status = "cancelled".into();
                        message.error = Some("The answer was cancelled before it finished.".into());
                    }
                }
            }
        }
        for doc in state
            .cache
            .values_mut()
            .filter(|doc| doc.scope_key == old_key)
        {
            doc.scope = scope.clone();
            doc.scope_key = new_key.clone();
            doc.persisted = false;
            doc.dirty = true;
        }
        if let Some(selected) = state.selected.remove(old_key) {
            state.selected.insert(new_key.clone(), selected);
            state.dirty_selection.insert(new_key.clone());
        }
        state.dirty_selection.remove(old_key);
        for (key, _, _) in state.used.values_mut() {
            if key == old_key {
                *key = new_key.clone();
            }
        }
        for key in state.pending_deletes.values_mut() {
            if key == old_key {
                *key = new_key.clone();
            }
        }
        let cancelled = state
            .cancelled_admissions
            .drain()
            .map(|(key, conversation, request)| {
                (
                    if key == old_key { new_key.clone() } else { key },
                    conversation,
                    request,
                )
            })
            .collect();
        state.cancelled_admissions = cancelled;
        let pending = state
            .pending_cancelled_admissions
            .drain()
            .map(|(key, conversation, request)| {
                (
                    if key == old_key { new_key.clone() } else { key },
                    conversation,
                    request,
                )
            })
            .collect();
        state.pending_cancelled_admissions = pending;
    }
    fn open_conversation_store(&self, state: &mut State, scope: &Scope) -> Option<Store> {
        if scope.uid().is_some_and(|uid| uid.starts_with("legacy-")) {
            state.storage_error=Some("This meeting's identity could not be saved. Conversations remain temporary; retry saving or copy text before quitting.".into());
            None
        } else {
            state.open(&self.data_dir())
        }
    }
    fn conversation_meeting_id(&self, uid: &str) -> Option<String> {
        self.meeting_id_for_uid(uid).or_else(|| {
            if !uid.starts_with("legacy-") {
                return None;
            }
            let store = self.meeting_store();
            store.list().into_iter().find_map(|meeting| {
                store
                    .read_transcript_snapshot(&meeting.id)
                    .ok()
                    .filter(|snapshot| snapshot.meeting_uid == uid)
                    .map(|_| meeting.id)
            })
        })
    }

    fn conversation_load(
        &self,
        state: &mut State,
        store: &mut Option<Store>,
        scope: &Scope,
        key: &str,
        requested: Option<&str>,
    ) -> Result<String, PennelError> {
        let selected = state.selected.get(key).cloned().or_else(|| {
            store
                .as_ref()
                .and_then(|store| store.selected(key).ok().flatten())
        });
        let id = requested.map(str::to_owned).or(selected);
        if let Some(id) = id.filter(|id| !state.deleted.contains(id)) {
            if let Some(doc) = state.cache.get(&id) {
                if doc.scope_key != key {
                    return Err(invalid(
                        "This conversation belongs to another meeting or library.",
                    ));
                }
                state
                    .selected
                    .entry(key.into())
                    .or_insert_with(|| id.clone());
                return Ok(id);
            }
            if let Some(doc) = store
                .as_ref()
                .map(|store| store.load(key, &id))
                .transpose()?
                .flatten()
            {
                state.room(&id, doc.size())?;
                state.cache.insert(id.clone(), doc);
                state
                    .selected
                    .entry(key.into())
                    .or_insert_with(|| id.clone());
                return Ok(id);
            }
            if requested.is_some() {
                return Err(if store.is_none() {
                    invalid(STORAGE_ERROR)
                } else {
                    PennelError::NotFound
                });
            }
        }
        if requested.is_some() {
            return Err(PennelError::NotFound);
        }
        let listed = store
            .as_ref()
            .map(|store| store.list(key, None))
            .transpose()?
            .unwrap_or_default();
        if let Some(summary) = listed.into_iter().find(|s| !state.deleted.contains(&s.id)) {
            let id = summary.id;
            let doc = store
                .as_ref()
                .unwrap()
                .load(key, &id)?
                .ok_or(PennelError::NotFound)?;
            state.room(&id, doc.size())?;
            state.cache.insert(id.clone(), doc);
            state.selected.insert(key.into(), id.clone());
            state.dirty_selection.insert(key.into());
            state.persist(store.as_mut(), &id);
            return Ok(id);
        }
        let doc = Cached::new(scope.clone(), key.into(), String::new());
        let id = doc.summary.id.clone();
        state.room(&id, doc.size())?;
        state.cache.insert(id.clone(), doc);
        state.selected.insert(key.into(), id.clone());
        state.dirty_selection.insert(key.into());
        state.persist(store.as_mut(), &id);
        Ok(id)
    }

    fn conversation_view(
        &self,
        state: &mut State,
        store: &mut Option<Store>,
        scope: Scope,
        library: &str,
        request: &ReadRequest,
    ) -> Result<Snapshot, PennelError> {
        let key = scope.key(library);
        let loaded = self.conversation_load(
            state,
            store,
            &scope,
            &key,
            request.conversation_id.as_deref(),
        );
        let id = match loaded {
            Ok(id) => id,
            Err(error) => {
                let initial = request.conversation_id.is_none()
                    && request.messages_before.is_none()
                    && request.conversations_before.is_none()
                    && request.focus_message_id.is_none();
                let Some(reason) = initial
                    .then(|| conversation_read_recovery_reason(&error))
                    .flatten()
                else {
                    return Err(error);
                };
                // A broken selected record must not disable ordinary Ask. This fresh recovery
                // context stays unsaved on read; requests naming an existing ID never retarget.
                let recovery = Cached::new(scope.clone(), key.clone(), String::new());
                let id = recovery.summary.id.clone();
                state.room(&id, recovery.size())?;
                state.cache.insert(id.clone(), recovery);
                state.selected.insert(key.clone(), id.clone());
                state.dirty_selection.insert(key.clone());
                state.storage_error = Some(reason);
                id
            }
        };
        let doc = state.cache.get(&id).ok_or(PennelError::NotFound)?;
        let saved_summaries = store
            .as_ref()
            .map(|store| store.list(&key, request.conversations_before.as_deref()))
            .transpose();
        let mut summaries = match saved_summaries {
            Ok(summaries) => summaries.unwrap_or_default(),
            Err(_) => {
                state.storage_error=Some("The saved conversation metadata could not be read. Its original data was kept.".into());
                Vec::new()
            }
        };
        let cursor = request
            .conversations_before
            .as_deref()
            .map(parse_summary_cursor)
            .transpose()?;
        for cached in state
            .cache
            .values()
            .filter(|doc| doc.scope_key == key && !state.deleted.contains(&doc.summary.id))
        {
            if cursor.as_ref().is_some_and(|(time, id)| {
                cached.summary.updated_at_ms > *time
                    || (cached.summary.updated_at_ms == *time && cached.summary.id >= *id)
            }) {
                continue;
            }
            summaries.retain(|s| s.id != cached.summary.id);
            summaries.push(cached.summary.clone());
        }
        summaries.retain(|s| !state.deleted.contains(&s.id));
        summaries.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        let next_conversations_cursor =
            (summaries.len() > PAGE).then(|| summary_cursor(&summaries[PAGE - 1]));
        summaries.truncate(PAGE);
        let (mut messages, next_messages_cursor) =
            conversation_message_page(doc, store.as_ref(), request)?;
        let mut inspector = SourceInspector::new(self);
        for message in &mut messages {
            if let Some(evidence) = message.evidence.as_mut() {
                for source in &mut evidence.sources {
                    source.status = inspector.resolve(source).status;
                }
            }
        }
        let unsaved = store.is_none()
            || state.storage_error.is_some()
            || state.cache.values().any(|d| d.scope_key == key && d.dirty)
            || state.dirty_selection.contains(&key)
            || state.pending_deletes.values().any(|scope| scope == &key)
            || state
                .pending_cancelled_admissions
                .iter()
                .any(|(scope, _, _)| scope == &key);
        Ok(Snapshot {
            schema_version: 1,
            current_meeting_id: scope
                .uid()
                .and_then(|uid| self.conversation_meeting_id(uid)),
            scope,
            storage_status: if unsaved { "unsaved" } else { "saved" }.into(),
            storage_error: unsaved.then(|| {
                state
                    .storage_error
                    .clone()
                    .unwrap_or_else(|| STORAGE_ERROR.into())
            }),
            selected_conversation_id: state
                .selected
                .get(&key)
                .cloned()
                .unwrap_or_else(|| id.clone()),
            conversation: doc.summary.clone(),
            conversations: summaries,
            messages,
            next_messages_cursor,
            next_conversations_cursor,
        })
    }

    fn conversation_finish(&self, request_id: &str, result: Result<MeetingAnswer, PennelError>) {
        let mut state = self.conversations.lock().unwrap();
        let Some(job) = state.active.remove(request_id) else {
            return;
        };
        self.finish_inference(&job.cancellation);
        let Some(doc) = state.cache.get_mut(&job.conversation_id) else {
            return;
        };
        if doc.scope_key != job.scope_key
            || doc.summary.active_request_id.as_deref() != Some(request_id)
        {
            return;
        }
        let Some(message) = doc.messages.iter_mut().find(|m| m.id == job.assistant_id) else {
            return;
        };
        match result {
            Ok(answer) if !job.cancellation.is_cancelled() => {
                message.status = "completed".into();
                message.text = answer.text.clone();
                message.answer = Some(answer);
                message.error = None;
            }
            Ok(_) | Err(PennelError::Cancelled) => {
                message.status = "cancelled".into();
                message.error = Some("The answer was cancelled before it finished.".into());
            }
            Err(_) => {
                message.status = "failed".into();
                message.error = Some(
                    "The answer could not be completed. Your question was kept; try again.".into(),
                );
            }
        }
        doc.summary.active_request_id = None;
        doc.summary.updated_at_ms = now();
        doc.dirty = true;
        let mut store = self.open_conversation_store(&mut state, &job.scope);
        state.persist(store.as_mut(), &job.conversation_id);
    }

    pub(crate) fn discard_meeting_conversations(&self, uid: &str) {
        let Ok(library) = library(self) else { return };
        let scope = Scope::Meeting {
            meeting_uid: uid.into(),
            meeting_id: None,
        };
        let key = scope.key(&library);
        let mut state = self.conversations.lock().unwrap();
        state.deleted_scopes.insert(key.clone());
        state
            .pending_cancelled_admissions
            .retain(|(scope, _, _)| scope != &key);
        let jobs: Vec<_> = state
            .active
            .iter()
            .filter(|(_, j)| j.scope_key == key)
            .map(|(id, _)| id.clone())
            .collect();
        for id in jobs {
            if let Some(job) = state.active.remove(&id) {
                job.cancellation.cancel();
                self.finish_inference(&job.cancellation);
            }
        }
        let ids: Vec<_> = state
            .cache
            .iter()
            .filter(|(_, doc)| doc.scope_key == key)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            state.cache.remove(&id);
            state.deleted.insert(id);
        }
        state.selected.remove(&key);
        if scope.uid().is_some_and(|uid| uid.starts_with("legacy-")) {
            state.deleted_scopes.remove(&key);
        } else if let Some(mut store) = self.open_conversation_store(&mut state, &scope)
            && store.delete_scope(&key).is_ok()
        {
            state.deleted_scopes.remove(&key);
        }
    }
}

#[uniffi::export]
impl PennelEngine {
    pub fn conversation_snapshot_json(&self, request_json: String) -> Result<String, PennelError> {
        let request: ReadRequest = parse(&request_json)?;
        let (scope, library) = self.normalize_conversation_scope(request.scope.clone())?;
        let mut state = self.conversations.lock().unwrap();
        let mut store = self.open_conversation_store(&mut state, &scope);
        output(&self.conversation_view(&mut state, &mut store, scope, &library, &request)?)
    }
    pub fn update_conversation_json(&self, request_json: String) -> Result<String, PennelError> {
        let request: UpdateRequest = parse(&request_json)?;
        if ["select", "rename", "delete", "draft"].contains(&request.action.as_str())
            && request.conversation_id.as_deref().is_none_or(str::is_empty)
            || request.action == "rename" && request.title.is_none()
        {
            return Err(invalid("The conversation request is invalid."));
        }
        let (scope, library) =
            self.normalize_conversation_scope_retry(request.scope, request.action == "retry_save")?;
        let key = scope.key(&library);
        let mut state = self.conversations.lock().unwrap();
        let mut store = self.open_conversation_store(&mut state, &scope);
        let id = if request.action == "create" {
            let doc = Cached::new(scope.clone(), key.clone(), title(request.title)?);
            let id = doc.summary.id.clone();
            state.room(&id, doc.size())?;
            state.cache.insert(id.clone(), doc);
            state.selected.insert(key.clone(), id.clone());
            state.dirty_selection.insert(key.clone());
            state.persist(store.as_mut(), &id);
            id
        } else {
            let id = self.conversation_load(
                &mut state,
                &mut store,
                &scope,
                &key,
                request.conversation_id.as_deref(),
            )?;
            match request.action.as_str() {
                "select" => {
                    state.selected.insert(key.clone(), id.clone());
                    state.dirty_selection.insert(key.clone());
                    state.persist(store.as_mut(), &id);
                }
                "rename" => {
                    let doc = state.cache.get_mut(&id).unwrap();
                    doc.summary.title = title(request.title)?;
                    doc.summary.updated_at_ms = now();
                    doc.dirty = true;
                    state.persist(store.as_mut(), &id);
                }
                "draft" => {
                    let draft = request
                        .draft
                        .ok_or_else(|| invalid("The conversation draft is missing."))?;
                    if draft.len() > 64 * 1024 {
                        return Err(invalid(
                            "The conversation draft is too large. Copy it before shortening it.",
                        ));
                    }
                    state.room(&id, draft.len())?;
                    let doc = state.cache.get_mut(&id).unwrap();
                    if request.expected_draft_revision != Some(doc.summary.draft_revision) {
                        return Err(invalid(
                            "The conversation draft changed elsewhere. Your text was kept; reload before saving.",
                        ));
                    }
                    doc.summary.draft = draft;
                    doc.summary.draft_revision += 1;
                    doc.summary.updated_at_ms = now();
                    doc.dirty = true;
                    state.persist(store.as_mut(), &id);
                }
                "delete" => {
                    let jobs: Vec<_> = state
                        .active
                        .iter()
                        .filter(|(_, job)| job.conversation_id == id)
                        .map(|(id, _)| id.clone())
                        .collect();
                    for request in jobs {
                        if let Some(job) = state.active.remove(&request) {
                            job.cancellation.cancel();
                            self.finish_inference(&job.cancellation);
                        }
                    }
                    state.cache.remove(&id);
                    state.deleted.insert(id.clone());
                    state
                        .pending_cancelled_admissions
                        .retain(|(_, conversation, _)| conversation != &id);
                    if store
                        .as_mut()
                        .is_none_or(|store| store.delete(&key, &id).is_err())
                    {
                        state.pending_deletes.insert(id.clone(), key.clone());
                    }
                    if state.selected.get(&key) == Some(&id) {
                        state.selected.remove(&key);
                    }
                }
                "retry_save" => {
                    if let Some(store) = store.as_mut() {
                        let pending: Vec<_> = state
                            .pending_deletes
                            .iter()
                            .filter(|(_, scope)| **scope == key)
                            .map(|(id, _)| id.clone())
                            .collect();
                        for id in pending {
                            if store.delete(&key, &id).is_ok() {
                                state.pending_deletes.remove(&id);
                            }
                        }
                        let deleted_scopes: Vec<_> = state.deleted_scopes.iter().cloned().collect();
                        for key in deleted_scopes {
                            if store.delete_scope(&key).is_ok() {
                                state.deleted_scopes.remove(&key);
                            }
                        }
                    }
                    let dirty: Vec<_> = state
                        .cache
                        .iter()
                        .filter(|(_, doc)| doc.scope_key == key && doc.dirty)
                        .map(|(id, _)| id.clone())
                        .collect();
                    for dirty in dirty {
                        state.persist(store.as_mut(), &dirty);
                    }
                    if state.dirty_selection.contains(&key) {
                        state.persist(store.as_mut(), &id);
                    }
                    if let Some(store) = store.as_ref() {
                        let pending: Vec<_> = state
                            .pending_cancelled_admissions
                            .iter()
                            .filter(|(scope, _, _)| scope == &key)
                            .cloned()
                            .collect();
                        for admission in pending {
                            if store
                                .cancel_admission(&admission.0, &admission.1, &admission.2)
                                .is_ok()
                            {
                                state.pending_cancelled_admissions.remove(&admission);
                            }
                        }
                    }
                }
                _ => return Err(invalid("The conversation action is unavailable.")),
            }
            id
        };
        let read = ReadRequest {
            scope: scope.clone(),
            conversation_id: (request.action != "delete").then_some(id),
            messages_before: None,
            conversations_before: None,
            focus_message_id: None,
        };
        output(&self.conversation_view(&mut state, &mut store, scope, &library, &read)?)
    }

    pub fn begin_conversation_question_json(
        self: Arc<Self>,
        request_json: String,
        keys: ApiKeys,
    ) -> Result<String, PennelError> {
        let mut request: BeginRequest = parse(&request_json)?;
        let (scope, library) = self.normalize_conversation_scope(request.scope.clone())?;
        request.scope = scope.clone();
        if request.question.trim().is_empty() || request.question.len() > 64 * 1024 {
            return Err(invalid("Type a question within the supported size first."));
        }
        if request.request_id.is_empty()
            || request.request_id.len() > 256
            || request.request_id.chars().any(char::is_control)
        {
            return Err(invalid("The question request identity is invalid."));
        }
        let key = scope.key(&library);
        let question_hash = crate::evidence_revision(&request.question);
        let cancellation = InferenceCancellation::new();
        let epoch = self.register_conversation_inference(&cancellation);
        let session = scope
            .uid()
            .and_then(|uid| self.conversation_live_session(uid));
        let session_id = session
            .as_ref()
            .map(|session| session.preview_snapshot().session_id);
        if request
            .session_id
            .as_ref()
            .is_some_and(|requested| Some(requested) != session_id.as_ref())
        {
            self.finish_inference(&cancellation);
            return Err(invalid(
                "This question belongs to another recording session.",
            ));
        }
        let (history, job) = {
            let mut state = self.conversations.lock().unwrap();
            let mut store = self.open_conversation_store(&mut state, &scope);
            let id = self.conversation_load(
                &mut state,
                &mut store,
                &scope,
                &key,
                Some(&request.conversation_id),
            )?;
            let used = if let Some(used) = state.used.get(&request.request_id) {
                Some(used.clone())
            } else {
                store
                    .as_ref()
                    .map(|s| s.used(&request.request_id))
                    .transpose()?
                    .flatten()
            };
            if let Some((old_scope, old_conversation, old_hash)) = used {
                self.finish_inference(&cancellation);
                if old_scope != key || old_conversation != id || old_hash != question_hash {
                    return Err(invalid(
                        "This question request belongs to another conversation or question.",
                    ));
                }
                let read = ReadRequest {
                    scope: scope.clone(),
                    conversation_id: Some(id),
                    messages_before: None,
                    conversations_before: None,
                    focus_message_id: None,
                };
                return output(
                    &self.conversation_view(&mut state, &mut store, scope, &library, &read)?,
                );
            }
            let admission = (key.clone(), id.clone(), request.request_id.clone());
            if state.cancelled_admissions.contains(&admission)
                || store
                    .as_ref()
                    .map(|store| store.admission_cancelled(&key, &id, &request.request_id))
                    .transpose()?
                    .unwrap_or(false)
            {
                self.finish_inference(&cancellation);
                return Err(PennelError::Cancelled);
            }
            if state.cache[&id].summary.active_request_id.is_some() {
                self.finish_inference(&cancellation);
                return Err(invalid(
                    "An answer is already running in this conversation. Cancel it or wait.",
                ));
            }
            let reserve = (state.active.len() + 1) * 1024 * 1024;
            state.room(&id, reserve)?;
            let doc = state.cache.get_mut(&id).unwrap();
            let history = doc.history();
            let position = doc.messages.last().map_or(0, |m| m.position + 1);
            let timestamp = now();
            let user_id = crate::conversations::id();
            let assistant_id = crate::conversations::id();
            doc.messages.push(Message {
                id: user_id.clone(),
                conversation_id: doc.summary.id.clone(),
                request_id: request.request_id.clone(),
                role: "user".into(),
                text: request.question.clone(),
                status: "completed".into(),
                created_at_ms: timestamp,
                reply_to: None,
                answer: None,
                evidence: None,
                error: None,
                position,
            });
            doc.messages.push(Message {
                id: assistant_id.clone(),
                conversation_id: doc.summary.id.clone(),
                request_id: request.request_id.clone(),
                role: "assistant".into(),
                text: String::new(),
                status: "pending".into(),
                created_at_ms: timestamp,
                reply_to: Some(user_id),
                answer: None,
                evidence: None,
                error: None,
                position: position + 1,
            });
            if doc.summary.draft.trim() == request.question.trim() {
                doc.summary.draft.clear();
                doc.summary.draft_revision += 1;
            }
            doc.summary.active_request_id = Some(request.request_id.clone());
            doc.summary.updated_at_ms = timestamp;
            doc.dirty = true;
            let job = ActiveConversationRequest {
                conversation_id: request.conversation_id.clone(),
                scope_key: key.clone(),
                library: library.clone(),
                scope: scope.clone(),
                assistant_id,
                cancellation: cancellation.clone(),
                session: session.as_ref().map(Arc::downgrade),
                session_id: session_id.clone(),
                epoch,
            };
            state.used.insert(
                request.request_id.clone(),
                (key.clone(), request.conversation_id.clone(), question_hash),
            );
            state.active.insert(request.request_id.clone(), job.clone());
            state.persist(store.as_mut(), &request.conversation_id);
            (history, job)
        };
        // Freeze acknowledged notes, finalized rows and cutoffs before returning admission. No network
        // runs while the conversation mutex or a capture/session lock is held.
        let frozen = self.freeze_conversation(&request, &history, &job);
        if let Ok(frozen) = frozen {
            {
                let mut state = self.conversations.lock().unwrap();
                if state.active.contains_key(&request.request_id) {
                    if let Some(doc) = state.cache.get_mut(&request.conversation_id)
                        && let Some(message) =
                            doc.messages.iter_mut().find(|m| m.id == job.assistant_id)
                    {
                        message.evidence = Some(frozen.evidence.clone());
                        doc.dirty = true;
                    }
                    let mut store = self.open_conversation_store(&mut state, &scope);
                    state.persist(store.as_mut(), &request.conversation_id);
                }
            }
            let engine = self.clone();
            let request_copy = request.clone();
            if std::thread::Builder::new()
                .name("pennel-conversation-question".into())
                .spawn(move || {
                    let result =
                        engine.run_conversation(&request_copy, history, frozen, &job, keys);
                    engine.conversation_finish(&request_copy.request_id, result);
                })
                .is_err()
            {
                self.conversation_finish(
                    &request.request_id,
                    Err(invalid("The question worker could not start.")),
                );
            }
        } else {
            self.conversation_finish(&request.request_id, Err(frozen.err().unwrap()));
        }
        self.conversation_snapshot_json(output(&ReadRequest {
            scope,
            conversation_id: Some(request.conversation_id),
            messages_before: None,
            conversations_before: None,
            focus_message_id: None,
        })?)
    }

    pub fn cancel_conversation_question_json(
        &self,
        request_json: String,
    ) -> Result<String, PennelError> {
        let request: CancelRequest = parse(&request_json)?;
        let (scope, library) = self.normalize_conversation_scope(request.scope)?;
        let key = scope.key(&library);
        let token = {
            let mut state = self.conversations.lock().unwrap();
            let mut store = self.open_conversation_store(&mut state, &scope);
            self.conversation_load(
                &mut state,
                &mut store,
                &scope,
                &key,
                Some(&request.conversation_id),
            )?;
            let token = state
                .active
                .get(&request.request_id)
                .filter(|job| {
                    job.scope_key == key && job.conversation_id == request.conversation_id
                })
                .map(|job| job.cancellation.clone());
            let used = if let Some(used) = state.used.get(&request.request_id) {
                Some(used.clone())
            } else {
                store
                    .as_ref()
                    .map(|store| store.used(&request.request_id))
                    .transpose()?
                    .flatten()
            };
            if token.is_none() && used.is_none() {
                let admission = (
                    key.clone(),
                    request.conversation_id.clone(),
                    request.request_id.clone(),
                );
                state.cancelled_admissions.insert(admission.clone());
                if store.as_ref().is_none_or(|store| {
                    store
                        .cancel_admission(&key, &request.conversation_id, &request.request_id)
                        .is_err()
                }) {
                    state.pending_cancelled_admissions.insert(admission);
                }
            }
            token
        };
        if let Some(token) = token {
            token.cancel();
            self.conversation_finish(&request.request_id, Err(PennelError::Cancelled));
        }
        self.conversation_snapshot_json(output(&ReadRequest {
            scope,
            conversation_id: Some(request.conversation_id),
            messages_before: None,
            conversations_before: None,
            focus_message_id: None,
        })?)
    }

    pub fn conversation_source_json(&self, request_json: String) -> Result<String, PennelError> {
        let request: SourceRequest = parse(&request_json)?;
        let (scope, library) = self.normalize_conversation_scope(request.scope)?;
        let key = scope.key(&library);
        let source = {
            let mut state = self.conversations.lock().unwrap();
            let mut store = self.open_conversation_store(&mut state, &scope);
            self.conversation_load(
                &mut state,
                &mut store,
                &scope,
                &key,
                Some(&request.conversation_id),
            )?;
            let message = state.cache[&request.conversation_id]
                .messages
                .iter()
                .find(|m| m.id == request.message_id)
                .cloned()
                .or_else(|| {
                    store.as_ref().and_then(|s| {
                        s.message(&request.conversation_id, &request.message_id)
                            .ok()
                            .flatten()
                    })
                })
                .ok_or(PennelError::NotFound)?;
            message
                .evidence
                .ok_or(PennelError::NotFound)?
                .sources
                .into_iter()
                .find(|source| source.number == request.citation_number)
                .ok_or(PennelError::NotFound)?
        };
        output(&SourceInspector::new(self).resolve(&source))
    }
}

impl PennelEngine {
    fn freeze_conversation(
        &self,
        request: &BeginRequest,
        history: &[crate::ChatTurn],
        job: &ActiveConversationRequest,
    ) -> Result<FrozenContext, PennelError> {
        if job.cancellation.is_cancelled() || library(self)? != job.library {
            return Err(PennelError::Cancelled);
        }
        let store = self.meeting_store();
        let mut transcript_revision = 0;
        let mut pending = 0;
        let mut frozen_current = None;
        let mut base = if let Some(uid) = request.scope.uid() {
            let id = self
                .conversation_meeting_id(uid)
                .ok_or(PennelError::NotFound)?;
            let info = store.info(&id).ok_or(PennelError::NotFound)?;
            let notes = self.meeting_notes_checked(&id)?;
            if request
                .expected_notes_revision
                .as_ref()
                .is_some_and(|hash| *hash != crate::evidence_revision(&notes))
            {
                return Err(invalid(
                    "The saved notes changed. Your question was not sent; save your notes and try again.",
                ));
            }
            let mut metadata_status = crate::TranscriptMetadataStatus::Valid;
            let rows = if let Some(session) = job
                .session
                .as_ref()
                .and_then(Weak::upgrade)
                .filter(|s| !s.is_finalized())
            {
                let (revision, rows) = session.utterance_snapshot();
                transcript_revision = revision;
                pending = session.pending();
                rows
            } else {
                let snapshot = store.read_transcript_snapshot(&id)?;
                transcript_revision = snapshot.revision;
                metadata_status = snapshot.metadata_status;
                if !matches!(
                    snapshot.metadata_status,
                    crate::TranscriptMetadataStatus::Valid
                        | crate::TranscriptMetadataStatus::Absent
                ) {
                    return Err(invalid(
                        "The finalized transcript's source metadata is unavailable.",
                    ));
                }
                snapshot.utterances
            };
            let names = store.speaker_names(&id);
            let summary = store.read_meeting(&id);
            frozen_current = Some((
                uid.to_owned(),
                CurrentMeeting {
                    id: id.clone(),
                    notes: Some(notes.clone()),
                    summary: summary.clone(),
                    rows: Some(rows.clone()),
                    names: names.clone(),
                    metadata_status,
                },
            ));
            let mut evidence = MeetingEvidence::new_with_utterances(
                uid,
                &info.title,
                &info.start_iso,
                &notes,
                &rows,
                &names,
            )?;
            if let Some(summary) = summary {
                let passage = Passage {
                    meeting_id: id.clone(),
                    meeting_uid: Some(uid.into()),
                    title: info.title,
                    start_iso: info.start_iso,
                    kind: PassageKind::Summary,
                    start_ms: None,
                    snippet: summary.chars().take(180).collect(),
                    text: summary.clone(),
                    utterance_ids: vec![],
                    utterance_fingerprints: vec![],
                };
                evidence.sources.push(passage.clone());
                evidence.anchors.push(MeetingSourceAnchor {
                    passage,
                    segment_keys: vec![],
                    segment_fingerprints: vec![],
                    notes_start: None,
                    notes_end: None,
                    source_text: summary,
                });
            }
            evidence
        } else {
            let settings = self.settings();
            let (account, _) = settings
                .resolve(&settings.llm)
                .ok_or(PennelError::NotFound)?;
            let select = || -> Result<Vec<Passage>, PennelError> {
                self.with_index(&store, |index| {
                    crate::meeting_chat::select_sources(
                        index,
                        &store,
                        &request.question,
                        history,
                        None,
                        account.kind == crate::ProviderKind::AppleIntelligence,
                    )
                    .map(|(sources, _)| sources)
                })
                .ok_or_else(|| invalid("The meeting search index is unavailable."))
            };
            let selected = select()?;
            for passage in &selected {
                if passage.meeting_uid.is_none()
                    || passage
                        .meeting_uid
                        .as_ref()
                        .is_some_and(|uid| uid.starts_with("legacy-"))
                {
                    let legacy = store
                        .read_transcript_snapshot(&passage.meeting_id)?
                        .meeting_uid;
                    self.normalize_conversation_scope(Scope::Meeting {
                        meeting_uid: legacy,
                        meeting_id: Some(passage.meeting_id.clone()),
                    })?;
                }
            }
            let mut selected = select()?;
            for passage in &mut selected {
                if passage.meeting_uid.is_none() {
                    passage.meeting_uid = Some(
                        store
                            .read_transcript_snapshot(&passage.meeting_id)?
                            .meeting_uid,
                    );
                }
            }
            let mut anchors = vec![];
            for passage in &selected {
                let notes = (passage.kind == PassageKind::Notes)
                    .then(|| self.meeting_notes_checked(&passage.meeting_id))
                    .transpose()?;
                let start = notes.as_ref().and_then(|notes| {
                    notes
                        .find(&passage.text)
                        .map(|byte| notes[..byte].encode_utf16().count())
                });
                anchors.push(MeetingSourceAnchor {
                    passage: passage.clone(),
                    segment_keys: passage
                        .utterance_ids
                        .iter()
                        .map(|id| format!("utterance:{id}"))
                        .collect(),
                    segment_fingerprints: passage.utterance_fingerprints.clone(),
                    notes_start: start,
                    notes_end: start.map(|start| start + passage.text.encode_utf16().count()),
                    source_text: passage.text.clone(),
                });
            }
            MeetingEvidence {
                sources: selected,
                anchors,
                notes_revision: String::new(),
                included_through_ms: 0,
            }
        };
        let available_sources = base.sources.len();
        let settings = self.settings();
        let (account, _) = settings
            .resolve(&settings.llm)
            .ok_or(PennelError::NotFound)?;
        let selected = base.selected_sources(
            &request.question,
            account.kind == crate::ProviderKind::AppleIntelligence,
        );
        base.anchors = base.anchors_for_sources(&selected)?;
        base.sources = selected;
        let mut inspector = SourceInspector::new(self);
        if let Some((uid, current)) = frozen_current {
            inspector.current.insert(uid, Some(current));
        }
        let mut sources = vec![];
        for (index, passage) in base.sources.iter().enumerate() {
            if request.scope.uid().is_none() {
                let current = inspector.current(passage).ok_or(PennelError::NotFound)?;
                let text = match passage.kind {
                    PassageKind::Notes => current.notes.as_deref(),
                    PassageKind::Summary => current.summary.as_deref(),
                    PassageKind::Transcript => None,
                };
                if !crate::meeting_index::frozen_passage_matches(
                    passage,
                    current.rows.as_deref().unwrap_or(&[]),
                    &current.names,
                    current.metadata_status,
                    text,
                ) {
                    return Err(invalid(
                        "The question source could not be verified; it was not sent.",
                    ));
                }
            }
            let anchor = base.anchors.get(index).cloned();
            let (revision, cutoff) = inspector.revision(passage)?;
            sources.push(StoredSource {
                number: index as u32 + 1,
                passage: passage.clone(),
                anchor,
                status: "current".into(),
                included_through_ms: cutoff,
                revision,
            });
        }
        let selection = MeetingQuestionSelection {
            scope: if request.scope.uid().is_none() || sources.len() != available_sources {
                MeetingQuestionScope::SelectedExcerpts
            } else {
                MeetingQuestionScope::WholeMeeting
            },
            available_sources: available_sources as u32,
            included_sources: sources.len() as u32,
            available_history_turns: history.len() as u32,
            included_history_turns: history.len().min(6) as u32,
            history_reduced: history.len() > 6,
        };
        let evidence = Evidence {
            request_id: request.request_id.clone(),
            meeting_uid: request.scope.uid().map(str::to_owned),
            session_id: job.session_id.clone(),
            transcript_revision,
            notes_revision: base.notes_revision.clone(),
            included_through_ms: base.included_through_ms,
            pending_transcription: pending,
            anchors: base.anchors.clone(),
            sources,
            selection,
        };
        Ok(FrozenContext { base, evidence })
    }
    fn conversation_context_current(
        &self,
        job: &ActiveConversationRequest,
        sources: &[StoredSource],
    ) -> bool {
        if job.cancellation.is_cancelled()
            || library(self).ok().as_deref() != Some(job.library.as_str())
        {
            return false;
        }
        if let Some(uid) = job.scope.uid() {
            if self.conversation_meeting_id(uid).is_none() {
                return false;
            }
            if let Some(current) = self.conversation_live_session(uid)
                && (job
                    .session
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .is_none_or(|old| !Arc::ptr_eq(&old, &current))
                    || job.session_id.as_deref()
                        != Some(current.preview_snapshot().session_id.as_str()))
            {
                return false;
            }
        }
        let mut inspector = SourceInspector::new(self);
        sources
            .iter()
            .all(|source| inspector.resolve(source).status == "current")
    }
    fn run_conversation(
        self: &Arc<Self>,
        request: &BeginRequest,
        history: Vec<crate::ChatTurn>,
        mut frozen: FrozenContext,
        job: &ActiveConversationRequest,
        keys: ApiKeys,
    ) -> Result<MeetingAnswer, PennelError> {
        if !self.conversation_context_current(job, &frozen.evidence.sources) {
            return Err(PennelError::Cancelled);
        }
        let deadline = Instant::now() + Duration::from_secs(crate::meeting_chat::ASK_TIMEOUT_SECS);
        let admission = crate::ForegroundQuestionAdmission::for_meeting(
            job.cancellation.clone(),
            deadline,
            request.scope.uid().unwrap_or("library").into(),
        )
        .with_inference_epoch(job.epoch);
        let prepared = self.prepare_meeting_evidence_question_admitted(
            request.question.clone(),
            history,
            &frozen.base,
            keys,
            admission,
        )?;
        let selected = prepared.sources();
        let mut remaining = frozen.evidence.sources.iter().map(Some).collect::<Vec<_>>();
        let mut actual = vec![];
        for (number, passage) in selected.iter().enumerate() {
            let index = remaining
                .iter()
                .position(|source| source.is_some_and(|source| source.passage == *passage))
                .ok_or_else(|| {
                    invalid("The question source could not be verified; it was not sent.")
                })?;
            let mut source = remaining[index].take().unwrap().clone();
            source.number = number as u32 + 1;
            actual.push(source);
        }
        frozen.evidence.sources = actual;
        frozen.evidence.anchors = frozen
            .evidence
            .sources
            .iter()
            .filter_map(|source| source.anchor.clone())
            .collect();
        let available_sources = frozen.evidence.selection.available_sources;
        frozen.evidence.selection = prepared.selection();
        frozen.evidence.selection.available_sources = available_sources;
        if request.scope.uid().is_none()
            || frozen.evidence.selection.included_sources
                < frozen.evidence.selection.available_sources
        {
            frozen.evidence.selection.scope = MeetingQuestionScope::SelectedExcerpts;
        }
        {
            let mut state = self.conversations.lock().unwrap();
            if !state.active.contains_key(&request.request_id) {
                return Err(PennelError::Cancelled);
            }
            let doc = state
                .cache
                .get_mut(&request.conversation_id)
                .ok_or(PennelError::Cancelled)?;
            let message = doc
                .messages
                .iter_mut()
                .find(|m| m.id == job.assistant_id)
                .ok_or(PennelError::Cancelled)?;
            message.evidence = Some(frozen.evidence.clone());
            doc.dirty = true;
            let mut store = self.open_conversation_store(&mut state, &request.scope);
            state.persist(store.as_mut(), &request.conversation_id);
        }
        let weak = Arc::downgrade(self);
        let guarded = job.clone();
        let sources = frozen.evidence.sources;
        let prepared = prepared.with_conversation_guard(Arc::new(move || {
            weak.upgrade()
                .is_some_and(|engine| engine.conversation_context_current(&guarded, &sources))
        }));
        prepared.complete(job.cancellation.clone(), ())
    }
}

struct CurrentMeeting {
    id: String,
    notes: Option<String>,
    summary: Option<String>,
    rows: Option<Vec<crate::MeetingUtterance>>,
    names: Vec<crate::SpeakerName>,
    metadata_status: crate::TranscriptMetadataStatus,
}
struct SourceInspector<'a> {
    engine: &'a PennelEngine,
    current: HashMap<String, Option<CurrentMeeting>>,
}
impl<'a> SourceInspector<'a> {
    fn new(engine: &'a PennelEngine) -> Self {
        Self {
            engine,
            current: HashMap::new(),
        }
    }
    fn current(&mut self, passage: &Passage) -> Option<&CurrentMeeting> {
        let uid = passage
            .meeting_uid
            .as_deref()
            .unwrap_or(&passage.meeting_id)
            .to_owned();
        if !self.current.contains_key(&uid) {
            let meeting = self.engine.conversation_meeting_id(&uid).map(|id| {
                let store = self.engine.meeting_store();
                let (rows, metadata_status) =
                    if let Some(session) = self.engine.conversation_live_session(&uid) {
                        (
                            Some(session.utterance_snapshot().1),
                            crate::TranscriptMetadataStatus::Valid,
                        )
                    } else {
                        match store.read_transcript_snapshot(&id) {
                            Ok(snapshot) => (
                                matches!(
                                    snapshot.metadata_status,
                                    crate::TranscriptMetadataStatus::Valid
                                        | crate::TranscriptMetadataStatus::Absent
                                )
                                .then_some(snapshot.utterances),
                                snapshot.metadata_status,
                            ),
                            Err(_) => (None, crate::TranscriptMetadataStatus::Corrupt),
                        }
                    };
                CurrentMeeting {
                    notes: self.engine.meeting_notes_checked(&id).ok(),
                    summary: store.read_meeting(&id),
                    rows,
                    names: store.speaker_names(&id),
                    metadata_status,
                    id,
                }
            });
            self.current.insert(uid.clone(), meeting);
        }
        self.current.get(&uid)?.as_ref()
    }
    fn revision(&mut self, passage: &Passage) -> Result<(String, u64), PennelError> {
        let current = self.current(passage).ok_or(PennelError::NotFound)?;
        match passage.kind {
            PassageKind::Notes => Ok((
                crate::evidence_revision(current.notes.as_deref().ok_or(PennelError::NotFound)?),
                0,
            )),
            PassageKind::Summary => Ok((
                crate::evidence_revision(current.summary.as_deref().ok_or(PennelError::NotFound)?),
                0,
            )),
            PassageKind::Transcript => {
                let rows = current.rows.as_ref().ok_or(PennelError::NotFound)?;
                let mut parts = vec![];
                let mut cutoff = 0;
                if passage.utterance_ids.is_empty() {
                    return Err(PennelError::NotFound);
                }
                for id in &passage.utterance_ids {
                    let row = rows
                        .iter()
                        .find(|row| row.metadata.utterance_id == *id && !row.segment.failed)
                        .ok_or(PennelError::NotFound)?;
                    cutoff = cutoff.max(row.segment.end_ms);
                    let name = row.segment.voice.and_then(|voice| {
                        current
                            .names
                            .iter()
                            .find(|name| name.voice == voice)
                            .map(|name| name.name.clone())
                    });
                    parts.push(serde_json::json!([
                        row.metadata.utterance_id,
                        row.metadata.source_fingerprint,
                        row.metadata.attribution_revision,
                        row.metadata.attribution,
                        row.metadata.manual,
                        row.segment.speaker,
                        row.segment.voice,
                        name
                    ]));
                }
                Ok((
                    crate::evidence_revision(
                        &serde_json::to_string(&parts).map_err(PennelError::storage)?,
                    ),
                    cutoff,
                ))
            }
        }
    }
    fn resolve(&mut self, source: &StoredSource) -> SourceResolution {
        let current_meeting_id = self
            .current(&source.passage)
            .map(|current| current.id.clone());
        let status = match self.revision(&source.passage) {
            Ok((revision, _)) if revision == source.revision => "current",
            Ok(_) => "stale",
            Err(_) => "unavailable",
        };
        SourceResolution {
            status: status.into(),
            passage: source.passage.clone(),
            anchor: source.anchor.clone(),
            current_meeting_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::PennelEngine;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    struct Model {
        calls: AtomicUsize,
        open: Mutex<bool>,
        changed: Condvar,
        prompts: Mutex<Vec<Vec<crate::LanguageModelMessage>>>,
    }
    impl Model {
        fn release(&self) {
            *self.open.lock().unwrap() = true;
            self.changed.notify_all();
        }
    }
    impl crate::SystemLanguageModel for Model {
        fn complete(
            &self,
            messages: Vec<crate::LanguageModelMessage>,
            _: u64,
        ) -> Result<String, crate::PennelError> {
            self.prompts.lock().unwrap().push(messages);
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut open = self.open.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !*open {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(crate::PennelError::Cancelled);
                }
                open = self.changed.wait_timeout(open, remaining).unwrap().0;
            }
            Ok("A frozen answer [1].".into())
        }
    }
    fn configured(
        blocked: bool,
    ) -> (
        tempfile::TempDir,
        Arc<PennelEngine>,
        String,
        Value,
        Arc<Model>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        let account = crate::new_account(
            crate::ProviderKind::AppleIntelligence,
            settings.providers.clone(),
        );
        settings.llm = crate::ModelChoice {
            provider_id: account.id.clone(),
            model: "synthetic".into(),
        };
        settings.providers.push(account);
        engine.update_settings(settings).unwrap();
        let model = Arc::new(Model {
            calls: AtomicUsize::new(0),
            open: Mutex::new(!blocked),
            changed: Condvar::new(),
            prompts: Mutex::new(vec![]),
        });
        engine.set_system_language_model(model.clone());
        let store = engine.meeting_store();
        let id = store
            .create("Synthetic source", chrono::Local::now())
            .unwrap();
        store
            .write_transcript(&id, &[row(0, "Original frozen source")])
            .unwrap();
        let scope = json!({"kind":"meeting","meeting_uid":engine.meeting_uid(id.clone()).unwrap()});
        (dir, engine, id, scope, model)
    }
    fn row(start: u64, text: &str) -> crate::Segment {
        crate::Segment {
            start_ms: start,
            end_ms: start + 1000,
            speaker: crate::Speaker::Others,
            text: text.into(),
            voice: Some(1),
            failed: false,
        }
    }
    fn begin(
        engine: &Arc<PennelEngine>,
        scope: Value,
        conversation: Value,
        request: &str,
    ) -> Value {
        serde_json::from_str(&engine.clone().begin_conversation_question_json(json!({"scope":scope,"conversation_id":conversation,"request_id":request,"question":"What does the source say?"}).to_string(),crate::ApiKeys{by_provider:Default::default()}).unwrap()).unwrap()
    }
    fn completed(engine: &PennelEngine, scope: &Value, conversation: &Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let snapshot = read(
                engine,
                json!({"scope":scope,"conversation_id":conversation}),
            );
            if snapshot["conversation"]["active_request_id"].is_null() {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "synthetic conversation did not finish"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn started(model: &Model) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while model.calls.load(Ordering::SeqCst) == 0 {
            assert!(
                Instant::now() < deadline,
                "synthetic provider did not start"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn read(engine: &PennelEngine, request: Value) -> Value {
        serde_json::from_str(
            &engine
                .conversation_snapshot_json(request.to_string())
                .unwrap(),
        )
        .unwrap()
    }
    fn update(engine: &PennelEngine, request: Value) -> Value {
        serde_json::from_str(
            &engine
                .update_conversation_json(request.to_string())
                .unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn conversation_selection_and_draft_survive_restart_without_touching_meeting_files() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let scope = json!({"kind":"library"});
        let first = read(&engine, json!({"scope":scope}));
        let id = first["conversation"]["id"].clone();
        update(
            &engine,
            json!({"scope":scope,"conversation_id":id,"action":"draft","draft":"An unsent question","expected_draft_revision":first["conversation"]["draft_revision"]}),
        );
        let second = update(
            &engine,
            json!({"scope":scope,"action":"create","title":"Another conversation"}),
        );
        assert_ne!(second["conversation"]["id"], id);
        update(
            &engine,
            json!({"scope":scope,"conversation_id":id,"action":"select"}),
        );
        drop(engine);
        let reopened =
            PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let restored = read(&reopened, json!({"scope":scope}));
        assert_eq!(restored["conversation"]["id"], id);
        assert_eq!(restored["conversation"]["draft"], "An unsent question");
        assert_eq!(restored["conversations"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn background_answer_stays_in_original_conversation_and_request_retry_is_idempotent() {
        let (_dir, engine, _meeting, _scope, model) = configured(true);
        let scope = json!({"kind":"library"});
        let first = read(&engine, json!({"scope":scope}));
        let original = first["conversation"]["id"].clone();
        let pending = begin(&engine, scope.clone(), original.clone(), "original-request");
        assert_eq!(pending["messages"][1]["status"], "pending");
        started(&model);
        let next = update(
            &engine,
            json!({"scope":scope,"action":"create","title":"Second"}),
        );
        let other = next["conversation"]["id"].clone();
        update(
            &engine,
            json!({"scope":scope,"action":"draft","conversation_id":other,"draft":"Keep second draft","expected_draft_revision":0}),
        );
        model.release();
        let done = completed(&engine, &scope, &original);
        assert_eq!(done["messages"][1]["status"], "completed");
        assert_eq!(done["selected_conversation_id"], other);
        let repeated = begin(&engine, scope.clone(), original.clone(), "original-request");
        assert_eq!(repeated["messages"].as_array().unwrap().len(), 2);
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        let selected = read(&engine, json!({"scope":scope}));
        assert_eq!(selected["conversation"]["id"], other);
        assert_eq!(selected["conversation"]["draft"], "Keep second draft");
        assert!(selected["messages"].as_array().unwrap().is_empty());
    }

    #[test]
    fn admitted_cutoff_survives_append_and_rename_while_speaker_changes_mark_sources_stale() {
        let (_dir, engine, meeting, scope, model) = configured(true);
        let initial = read(&engine, json!({"scope":scope}));
        let conversation = initial["conversation"]["id"].clone();
        let pending = begin(
            &engine,
            scope.clone(),
            conversation.clone(),
            "cutoff-request",
        );
        assert_eq!(
            pending["messages"][1]["evidence"]["included_through_ms"],
            1000
        );
        engine
            .meeting_store()
            .write_transcript(
                &meeting,
                &[
                    row(0, "Original frozen source"),
                    row(1000, "Added after admission"),
                ],
            )
            .unwrap();
        started(&model);
        model.release();
        let done = completed(&engine, &scope, &conversation);
        assert_eq!(done["messages"][1]["status"], "completed");
        assert_eq!(done["messages"][1]["evidence"]["included_through_ms"], 1000);
        assert!(
            model
                .prompts
                .lock()
                .unwrap()
                .iter()
                .flatten()
                .all(|message| !message.content.contains("Added after admission"))
        );
        let answer_id = done["messages"][1]["id"].clone();
        let renamed = engine
            .rename_meeting(meeting.clone(), "Renamed source".into())
            .unwrap();
        let hinted = read(
            &engine,
            json!({"scope":{"kind":"meeting","meeting_uid":scope["meeting_uid"],"meeting_id":meeting},"conversation_id":conversation}),
        );
        assert_eq!(hinted["current_meeting_id"], renamed);
        let source_request = json!({"scope":scope,"conversation_id":conversation,"message_id":answer_id,"citation_number":1});
        let current: Value = serde_json::from_str(
            &engine
                .conversation_source_json(source_request.to_string())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(current["status"], "current");
        assert_eq!(current["current_meeting_id"], renamed);
        engine
            .rename_speaker(renamed, 1, "New authored label".into())
            .unwrap();
        let stale: Value = serde_json::from_str(
            &engine
                .conversation_source_json(source_request.to_string())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(stale["status"], "stale");
        let retained = read(
            &engine,
            json!({"scope":scope,"conversation_id":conversation}),
        );
        assert_eq!(retained["messages"][1]["text"], "A frozen answer [1].");
    }

    #[test]
    fn cancelling_or_deleting_rejects_late_answers_without_changing_notes() {
        let (_dir, engine, meeting, scope, model) = configured(true);
        engine
            .set_meeting_notes(meeting.clone(), "Authored notes remain".into())
            .unwrap();
        let first = read(&engine, json!({"scope":scope}));
        let original = first["conversation"]["id"].clone();
        begin(
            &engine,
            scope.clone(),
            original.clone(),
            "cancelled-request",
        );
        started(&model);
        let cancelled:Value=serde_json::from_str(&engine.cancel_conversation_question_json(json!({"scope":scope,"conversation_id":original,"request_id":"cancelled-request"}).to_string()).unwrap()).unwrap();
        assert_eq!(cancelled["messages"][1]["status"], "cancelled");
        model.release();
        let removed = update(
            &engine,
            json!({"scope":scope,"conversation_id":original,"action":"delete"}),
        );
        assert_ne!(removed["conversation"]["id"], original);
        assert!(removed["messages"].as_array().unwrap().is_empty());
        assert!(
            engine
                .conversation_snapshot_json(
                    json!({"scope":scope,"conversation_id":original}).to_string()
                )
                .is_err()
        );
        assert_eq!(engine.meeting_notes(meeting), "Authored notes remain");
    }

    #[test]
    fn unavailable_store_keeps_questions_usable_and_retry_saves_without_duplicate_messages() {
        let (dir, engine, _meeting, scope, _model) = configured(false);
        std::fs::create_dir(dir.path().join("conversations.sqlite")).unwrap();
        let initial = read(&engine, json!({"scope":scope}));
        assert_eq!(initial["storage_status"], "unsaved");
        let conversation = initial["conversation"]["id"].clone();
        begin(
            &engine,
            scope.clone(),
            conversation.clone(),
            "unsaved-request",
        );
        let done = completed(&engine, &scope, &conversation);
        assert_eq!(done["messages"][1]["status"], "completed");
        assert_eq!(done["storage_status"], "unsaved");
        std::fs::remove_dir(dir.path().join("conversations.sqlite")).unwrap();
        let saved = update(
            &engine,
            json!({"scope":scope,"conversation_id":conversation,"action":"retry_save"}),
        );
        assert_eq!(saved["storage_status"], "saved");
        assert_eq!(saved["messages"].as_array().unwrap().len(), 2);
        let again = update(
            &engine,
            json!({"scope":scope,"conversation_id":conversation,"action":"retry_save"}),
        );
        assert_eq!(again["messages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn stale_draft_writes_and_cross_library_requests_preserve_original_text() {
        let dir = tempfile::tempdir().unwrap();
        let first_root = dir.path().join("First");
        let second_root = dir.path().join("Second");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        let mut settings = crate::settings::default_settings();
        settings.meetings_dir = first_root.to_string_lossy().into_owned();
        std::fs::write(
            dir.path().join("settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        let scope = json!({"kind":"library"});
        let initial = read(&engine, json!({"scope":scope}));
        let conversation = initial["conversation"]["id"].clone();
        update(
            &engine,
            json!({"scope":scope,"conversation_id":conversation,"action":"draft","draft":"Owner text","expected_draft_revision":0}),
        );
        assert!(engine.update_conversation_json(json!({"scope":scope,"conversation_id":conversation,"action":"draft","draft":"Late stale text","expected_draft_revision":0}).to_string()).is_err());
        settings.meetings_dir = second_root.to_string_lossy().into_owned();
        engine.update_settings(settings.clone()).unwrap();
        assert!(engine.update_conversation_json(json!({"scope":scope,"conversation_id":conversation,"action":"draft","draft":"Wrong library","expected_draft_revision":1}).to_string()).is_err());
        let other = read(&engine, json!({"scope":scope}));
        assert_ne!(other["conversation"]["id"], conversation);
        assert_eq!(other["conversation"]["draft"], "");
        settings.meetings_dir = first_root.to_string_lossy().into_owned();
        engine.update_settings(settings).unwrap();
        assert_eq!(
            read(&engine, json!({"scope":scope}))["conversation"]["draft"],
            "Owner text"
        );
    }

    #[test]
    fn legacy_meeting_identity_is_migrated_once_and_older_markdown_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let meeting = store.create("Legacy", chrono::Local::now()).unwrap();
        let path = store.dir_of(&meeting).join("meeting.md");
        let original = std::fs::read_to_string(&path).unwrap();
        let legacy = original
            .lines()
            .filter(|line| !line.starts_with("pennel_id:"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, &legacy).unwrap();
        let legacy_uid = engine
            .meeting_utterance_snapshot(meeting.clone())
            .unwrap()
            .meeting_uid;
        let migrated = read(
            &engine,
            json!({"scope":{"kind":"meeting","meeting_uid":legacy_uid,"meeting_id":meeting}}),
        );
        assert_ne!(migrated["scope"]["meeting_uid"], legacy_uid);
        assert_eq!(
            engine.meeting_uid(meeting.clone()).unwrap(),
            migrated["scope"]["meeting_uid"].as_str().unwrap()
        );
        let current = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            current
                .lines()
                .filter(|line| !line.starts_with("pennel_id:"))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
            legacy
        );
        assert_eq!(
            read(&engine, json!({"scope":migrated["scope"]}))["conversation"]["id"],
            migrated["conversation"]["id"]
        );
    }

    #[test]
    fn stopping_renaming_and_starting_another_recording_keep_the_original_pending_answer() {
        let (_dir, engine, _meeting, _scope, model) = configured(true);
        let session = engine
            .clone()
            .start_meeting_with(
                "Live source".into(),
                vec![],
                crate::ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        session.set_notes("A saved live source".into()).unwrap();
        let scope =
            json!({"kind":"meeting","meeting_uid":engine.meeting_uid(session.id()).unwrap()});
        let initial = read(&engine, json!({"scope":scope}));
        let conversation = initial["conversation"]["id"].clone();
        let pending = begin(
            &engine,
            scope.clone(),
            conversation.clone(),
            "live-finish-request",
        );
        assert_eq!(
            pending["messages"][1]["evidence"]["session_id"],
            session.preview_snapshot().session_id
        );
        started(&model);
        session.stop().unwrap();
        let renamed = engine
            .rename_meeting(session.id(), "Automatic title".into())
            .unwrap();
        let other = engine
            .clone()
            .start_meeting_with(
                "Another recording".into(),
                vec![],
                crate::ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        model.release();
        let done = completed(&engine, &scope, &conversation);
        assert_eq!(done["messages"][1]["status"], "completed");
        assert_eq!(done["current_meeting_id"], renamed);
        other.stop().unwrap();
    }

    #[test]
    fn meeting_deletion_preserves_library_answers_with_unavailable_references() {
        let (_dir, engine, meeting, _scope, _model) = configured(false);
        let scope = json!({"kind":"library"});
        let initial = read(&engine, json!({"scope":scope}));
        let conversation = initial["conversation"]["id"].clone();
        begin(
            &engine,
            scope.clone(),
            conversation.clone(),
            "library-source-request",
        );
        let done = completed(&engine, &scope, &conversation);
        assert_eq!(done["messages"][1]["status"], "completed");
        let answer_id = done["messages"][1]["id"].clone();
        engine.delete_meeting(meeting).unwrap();
        let retained = read(
            &engine,
            json!({"scope":scope,"conversation_id":conversation}),
        );
        assert_eq!(retained["messages"][1]["text"], "A frozen answer [1].");
        let source:Value=serde_json::from_str(&engine.conversation_source_json(json!({"scope":scope,"conversation_id":conversation,"message_id":answer_id,"citation_number":1}).to_string()).unwrap()).unwrap();
        assert_eq!(source["status"], "unavailable");
        assert_eq!(source["current_meeting_id"], Value::Null);
    }

    #[test]
    fn deleting_a_meeting_during_inference_never_resurrects_its_conversation() {
        let (_dir, engine, meeting, scope, model) = configured(true);
        let first = read(&engine, json!({"scope":scope}));
        let conversation = first["conversation"]["id"].clone();
        begin(
            &engine,
            scope.clone(),
            conversation.clone(),
            "deleted-meeting-request",
        );
        started(&model);
        engine.delete_meeting(meeting).unwrap();
        model.release();
        assert!(
            engine
                .conversation_snapshot_json(
                    json!({"scope":scope,"conversation_id":conversation}).to_string()
                )
                .is_err()
        );
        assert!(engine.meetings().is_empty());
    }

    #[test]
    fn future_schema_remains_unchanged_while_unsaved_questions_stay_usable() {
        let (dir, engine, _meeting, scope, _model) = configured(false);
        let first = read(&engine, json!({"scope":scope}));
        let conversation = first["conversation"]["id"].clone();
        let path = dir.path().join("conversations.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch("PRAGMA user_version=9000;")
            .unwrap();
        drop(connection);
        let original = std::fs::read(&path).unwrap();
        let unavailable = read(
            &engine,
            json!({"scope":scope,"conversation_id":conversation}),
        );
        assert_eq!(unavailable["storage_status"], "unsaved");
        assert!(
            unavailable["storage_error"]
                .as_str()
                .unwrap()
                .contains("newer Pennel")
        );
        begin(
            &engine,
            scope.clone(),
            conversation.clone(),
            "future-schema-request",
        );
        let done = completed(&engine, &scope, &conversation);
        assert_eq!(done["messages"][1]["status"], "completed");
        assert_eq!(done["storage_status"], "unsaved");
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn cancel_before_admission_blocks_only_the_matching_conversation_and_request() {
        let (dir, engine, _meeting, scope, model) = configured(false);
        let first = read(&engine, json!({"scope":scope}));
        let conversation = first["conversation"]["id"].clone();
        update(
            &engine,
            json!({"scope":scope,"conversation_id":conversation,"action":"draft","draft":"What does the source say?","expected_draft_revision":0}),
        );
        engine.cancel_conversation_question_json(json!({"scope":scope,"conversation_id":conversation,"request_id":"overtaken-request"}).to_string()).unwrap();
        drop(engine);
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        engine.set_system_language_model(model.clone());
        let cancelled=engine.clone().begin_conversation_question_json(json!({"scope":scope,"conversation_id":conversation,"request_id":"overtaken-request","question":"What does the source say?"}).to_string(),crate::ApiKeys{by_provider:Default::default()});
        assert!(matches!(cancelled, Err(crate::PennelError::Cancelled)));
        let kept = read(
            &engine,
            json!({"scope":scope,"conversation_id":conversation}),
        );
        assert_eq!(kept["conversation"]["draft"], "What does the source say?");
        assert!(kept["messages"].as_array().unwrap().is_empty());
        assert_eq!(model.calls.load(Ordering::SeqCst), 0);
        let second = update(&engine, json!({"scope":scope,"action":"create"}));
        let other = second["conversation"]["id"].clone();
        begin(&engine, scope.clone(), other.clone(), "overtaken-request");
        let done = completed(&engine, &scope, &other);
        assert_eq!(done["messages"][1]["status"], "completed");
        begin(&engine, scope, other, "overtaken-request");
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn old_answer_focus_remains_bounded_and_readable_during_a_draft_save_failure() {
        let (dir, engine, _meeting, scope, _model) = configured(false);
        let initial = read(&engine, json!({"scope":scope}));
        let conversation = initial["conversation"]["id"].clone();
        let mut old_answer = Value::Null;
        for index in 0..28 {
            begin(
                &engine,
                scope.clone(),
                conversation.clone(),
                &format!("page-request-{index}"),
            );
            let done = completed(&engine, &scope, &conversation);
            if index == 0 {
                old_answer = done["messages"][1]["id"].clone();
            }
        }
        let latest = read(
            &engine,
            json!({"scope":scope,"conversation_id":conversation}),
        );
        assert_eq!(latest["messages"].as_array().unwrap().len(), 50);
        assert!(latest["next_messages_cursor"].is_string());
        let connection =
            rusqlite::Connection::open(dir.path().join("conversations.sqlite")).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_draft_save BEFORE UPDATE ON conversations BEGIN SELECT RAISE(FAIL,'injected draft save failure'); END;").unwrap();
        let unsaved = update(
            &engine,
            json!({"scope":scope,"conversation_id":conversation,"action":"draft","draft":"Keep the unsaved draft","expected_draft_revision":latest["conversation"]["draft_revision"]}),
        );
        assert_eq!(unsaved["storage_status"], "unsaved");
        let focused = read(
            &engine,
            json!({"scope":scope,"conversation_id":conversation,"focus_message_id":old_answer}),
        );
        assert!(focused["messages"].as_array().unwrap().len() <= 50);
        assert!(
            focused["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["id"] == old_answer)
        );
        assert_eq!(focused["conversation"]["draft"], "Keep the unsaved draft");
        connection
            .execute_batch("DROP TRIGGER fail_draft_save;")
            .unwrap();
    }

    #[test]
    fn corrupt_store_is_preserved_and_private_chat_never_enters_meeting_exports_or_sync() {
        let (dir, engine, meeting, scope, _model) = configured(false);
        let path = dir.path().join("conversations.sqlite");
        let corrupt = b"original unreadable conversation store\0keep this evidence";
        std::fs::write(&path, corrupt).unwrap();
        let initial = read(&engine, json!({"scope":scope}));
        assert_eq!(initial["storage_status"], "unsaved");
        let conversation = initial["conversation"]["id"].clone();
        engine.clone().begin_conversation_question_json(json!({"scope":scope,"conversation_id":conversation,"request_id":"private-export-request","question":"Private chat marker: what does the source say?"}).to_string(),crate::ApiKeys{by_provider:Default::default()}).unwrap();
        let done = completed(&engine, &scope, &conversation);
        assert_eq!(done["messages"][1]["status"], "completed");
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        let exported = engine
            .export_meeting(meeting.clone(), true)
            .unwrap()
            .markdown;
        assert!(!exported.contains("Private chat marker"));
        assert!(!exported.contains("A frozen answer"));
        let store = engine.meeting_store();
        let files = crate::cloud::sync::scan::scan(
            &store.root(),
            &[(engine.meeting_uid(meeting.clone()).unwrap(), meeting)]
                .into_iter()
                .collect(),
        )
        .unwrap();
        assert!(files.iter().all(|file| {
            !file
                .rel_path
                .to_string_lossy()
                .contains("conversations.sqlite")
        }));
    }
    fn legacy_meeting(engine: &PennelEngine, title: &str) -> (String, String) {
        let store = engine.meeting_store();
        let id = store.create(title, chrono::Local::now()).unwrap();
        store.write_notes(&id, "An exact legacy note").unwrap();
        let path = store.dir_of(&id).join("meeting.md");
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            path,
            original
                .lines()
                .filter(|line| !line.starts_with("pennel_id:"))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();
        let uid = engine
            .meeting_utterance_snapshot(id.clone())
            .unwrap()
            .meeting_uid;
        (id, uid)
    }
    #[test]
    fn concurrent_legacy_normalizations_share_only_the_identity_written_by_this_engine() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let (meeting, legacy) = legacy_meeting(&engine, "Concurrent legacy");
        let scope = json!({"kind":"meeting","meeting_uid":legacy,"meeting_id":meeting});
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let engine = engine.clone();
                let barrier = barrier.clone();
                let scope = scope.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    read(&engine, json!({"scope":scope}))
                })
            })
            .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results[0]["scope"], results[1]["scope"]);
        assert_eq!(
            results[0]["conversation"]["id"],
            results[1]["conversation"]["id"]
        );
        let canonical = engine.meeting_uid(meeting.clone()).unwrap();
        let body =
            std::fs::read_to_string(engine.meeting_store().dir_of(&meeting).join("meeting.md"))
                .unwrap();
        engine.delete_meeting(meeting.clone()).unwrap();
        let replacement = engine.meeting_store().dir_of(&meeting);
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::write(
            replacement.join("meeting.md"),
            body.replace(&canonical, &uuid::Uuid::now_v7().to_string()),
        )
        .unwrap();
        assert!(
            engine
                .conversation_snapshot_json(json!({"scope":scope}).to_string())
                .is_err()
        );
        assert!(engine.conversation_snapshot_json(json!({"scope":{"kind":"meeting","meeting_uid":"legacy-conflicting","meeting_id":meeting}}).to_string()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn read_only_legacy_questions_stay_temporary_and_explicit_retry_promotes_same_ids_and_draft() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, engine, _meeting, _scope, model) = configured(true);
        let (meeting, legacy) = legacy_meeting(&engine, "Read only legacy");
        let source_dir = engine.meeting_store().dir_of(&meeting);
        struct Restore(std::path::PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
            }
        }
        let _restore = Restore(source_dir.clone());
        std::fs::set_permissions(&source_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let scope = json!({"kind":"meeting","meeting_uid":legacy,"meeting_id":meeting});
        let initial = read(&engine, json!({"scope":scope}));
        assert_eq!(initial["storage_status"], "unsaved");
        assert!(
            initial["storage_error"]
                .as_str()
                .unwrap()
                .contains("identity")
        );
        assert!(!dir.path().join("conversations.sqlite").exists());
        let conversation = initial["conversation"]["id"].clone();
        begin(
            &engine,
            scope.clone(),
            conversation.clone(),
            "temporary-legacy-request",
        );
        started(&model);
        update(
            &engine,
            json!({"scope":scope,"conversation_id":conversation,"action":"draft","draft":"A later unsent draft","expected_draft_revision":initial["conversation"]["draft_revision"]}),
        );
        std::fs::set_permissions(&source_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let deferred = update(
            &engine,
            json!({"scope":scope,"conversation_id":conversation,"action":"retry_save"}),
        );
        assert_eq!(deferred["scope"]["meeting_uid"], legacy);
        assert!(engine.meeting_uid(meeting.clone()).is_none());
        model.release();
        let done = completed(&engine, &scope, &conversation);
        assert_eq!(done["messages"][1]["status"], "completed");
        let old_ids: Vec<_> = done["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["id"].clone())
            .collect();
        let promoted = update(
            &engine,
            json!({"scope":scope,"conversation_id":conversation,"action":"retry_save"}),
        );
        assert_eq!(promoted["storage_status"], "saved");
        assert_ne!(promoted["scope"]["meeting_uid"], legacy);
        assert_eq!(promoted["conversation"]["id"], conversation);
        assert_eq!(promoted["conversation"]["draft"], "A later unsent draft");
        assert_eq!(
            promoted["messages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|message| message["id"].clone())
                .collect::<Vec<_>>(),
            old_ids
        );
        assert_eq!(engine.meeting_notes(meeting), "An exact legacy note");
    }
    #[test]
    fn processing_configuration_change_cancels_the_frozen_request_without_retargeting() {
        let (_dir, engine, _meeting, scope, model) = configured(true);
        let first = read(&engine, json!({"scope":scope}));
        let conversation = first["conversation"]["id"].clone();
        begin(
            &engine,
            scope.clone(),
            conversation.clone(),
            "changed-provider-request",
        );
        started(&model);
        let mut settings = engine.settings();
        settings.llm.model = "new synthetic model".into();
        engine.update_settings(settings).unwrap();
        model.release();
        let done = completed(&engine, &scope, &conversation);
        assert_eq!(done["messages"][1]["status"], "cancelled");
        assert_eq!(done["messages"][1]["text"], "");
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn corrupt_summary_identity_cannot_redirect_public_mutations_to_another_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let scope = json!({"kind":"library"});
        let first = read(&engine, json!({"scope":scope}));
        let a = first["conversation"]["id"].clone();
        let second = update(
            &engine,
            json!({"scope":scope,"action":"create","title":"B original title"}),
        );
        let b = second["conversation"]["id"].clone();
        update(
            &engine,
            json!({"scope":scope,"conversation_id":b,"action":"draft","draft":"B exact draft","expected_draft_revision":0}),
        );
        drop(engine);
        let connection =
            rusqlite::Connection::open(dir.path().join("conversations.sqlite")).unwrap();
        connection
            .execute(
                "UPDATE conversations SET summary=json_set(summary,'$.id',?1) WHERE id=?2",
                rusqlite::params![b.as_str().unwrap(), a.as_str().unwrap()],
            )
            .unwrap();
        let corrupt: String = connection
            .query_row(
                "SELECT summary FROM conversations WHERE id=?1",
                [a.as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        assert!(engine.update_conversation_json(json!({"scope":scope,"conversation_id":a,"action":"rename","title":"Wrong B title"}).to_string()).is_err());
        assert!(engine.update_conversation_json(json!({"scope":scope,"conversation_id":a,"action":"draft","draft":"Wrong B draft","expected_draft_revision":0}).to_string()).is_err());
        let intact = read(&engine, json!({"scope":scope,"conversation_id":b}));
        assert_eq!(intact["conversation"]["title"], "B original title");
        assert_eq!(intact["conversation"]["draft"], "B exact draft");
        assert_eq!(intact["storage_status"], "unsaved");
        let retained: String = connection
            .query_row(
                "SELECT summary FROM conversations WHERE id=?1",
                [a.as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, corrupt);
        assert!(
            engine
                .update_conversation_json(json!({"scope":scope,"action":"delete"}).to_string())
                .is_err()
        );
        assert_eq!(
            read(&engine, json!({"scope":scope,"conversation_id":b}))["conversation"]["draft"],
            "B exact draft"
        );
    }
    enum SelectedCorruption {
        Summary,
        Message,
        Generation,
    }
    fn corrupt_selected_recovery(corruption: SelectedCorruption) {
        let corrupt_message = matches!(corruption, SelectedCorruption::Message);
        let (dir, engine, _meeting, scope, model) = configured(false);
        let first = read(&engine, json!({"scope":scope}));
        let a = first["conversation"]["id"].clone();
        let mut damaged_message = Value::Null;
        if corrupt_message {
            begin(
                &engine,
                scope.clone(),
                a.clone(),
                "original-corrupt-message",
            );
            let done = completed(&engine, &scope, &a);
            damaged_message = done["messages"][1]["id"].clone();
        }
        let second = update(
            &engine,
            json!({"scope":scope,"action":"create","title":"B must remain"}),
        );
        let b = second["conversation"]["id"].clone();
        update(
            &engine,
            json!({"scope":scope,"conversation_id":b,"action":"draft","draft":"B exact retained draft","expected_draft_revision":0}),
        );
        update(
            &engine,
            json!({"scope":scope,"conversation_id":a,"action":"select"}),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&engine) > 1 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(engine);
        let connection =
            rusqlite::Connection::open(dir.path().join("conversations.sqlite")).unwrap();
        if corrupt_message {
            connection.execute("UPDATE messages SET body=json_set(body,'$.request_id','corrupt-request') WHERE id=?1",[damaged_message.as_str().unwrap()]).unwrap();
        } else if matches!(corruption, SelectedCorruption::Generation) {
            connection
                .execute(
                    "UPDATE conversations SET generation=-1 WHERE id=?1",
                    [a.as_str().unwrap()],
                )
                .unwrap();
        } else {
            connection
                .execute(
                    "UPDATE conversations SET summary=json_set(summary,'$.id',?1) WHERE id=?2",
                    rusqlite::params![b.as_str().unwrap(), a.as_str().unwrap()],
                )
                .unwrap();
        }
        let original_generation: i64 = connection
            .query_row(
                "SELECT generation FROM conversations WHERE id=?1",
                [a.as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let original_summary: String = connection
            .query_row(
                "SELECT summary FROM conversations WHERE id=?1",
                [a.as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let original_body: Option<String> = if corrupt_message {
            Some(
                connection
                    .query_row(
                        "SELECT body FROM messages WHERE id=?1",
                        [damaged_message.as_str().unwrap()],
                        |row| row.get(0),
                    )
                    .unwrap(),
            )
        } else {
            None
        };
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        engine.set_system_language_model(model.clone());
        let recovery = read(&engine, json!({"scope":scope}));
        let recovered_id = recovery["conversation"]["id"].clone();
        assert_ne!(recovered_id, a);
        assert_ne!(recovered_id, b);
        assert_eq!(recovery["storage_status"], "unsaved");
        assert!(recovery["storage_error"].is_string());
        assert!(recovery["messages"].as_array().unwrap().is_empty());
        assert!(
            engine
                .conversation_snapshot_json(json!({"scope":scope,"conversation_id":a}).to_string())
                .is_err()
        );
        assert!(engine.update_conversation_json(json!({"scope":scope,"conversation_id":a,"action":"rename","title":"Do not change B"}).to_string()).is_err());
        assert!(engine.clone().begin_conversation_question_json(json!({"scope":scope,"conversation_id":a,"request_id":"explicit-corrupt-target","question":"Wrong old target"}).to_string(),crate::ApiKeys{by_provider:Default::default()}).is_err());
        assert!(engine.cancel_conversation_question_json(json!({"scope":scope,"conversation_id":a,"request_id":"explicit-corrupt-target"}).to_string()).is_err());
        assert!(engine.conversation_source_json(json!({"scope":scope,"conversation_id":a,"message_id":"missing-source","citation_number":1}).to_string()).is_err());
        begin(
            &engine,
            scope.clone(),
            recovered_id.clone(),
            "new-recovery-question",
        );
        let done = completed(&engine, &scope, &recovered_id);
        assert_eq!(done["messages"][1]["status"], "completed");
        let intact = read(&engine, json!({"scope":scope,"conversation_id":b}));
        assert_eq!(intact["conversation"]["title"], "B must remain");
        assert_eq!(intact["conversation"]["draft"], "B exact retained draft");
        let retained_summary: String = connection
            .query_row(
                "SELECT summary FROM conversations WHERE id=?1",
                [a.as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained_summary, original_summary);
        let retained_generation: i64 = connection
            .query_row(
                "SELECT generation FROM conversations WHERE id=?1",
                [a.as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained_generation, original_generation);
        if let Some(original_body) = original_body {
            let retained_body: String = connection
                .query_row(
                    "SELECT body FROM messages WHERE id=?1",
                    [damaged_message.as_str().unwrap()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(retained_body, original_body);
        }
    }
    #[test]
    fn corrupt_selected_summary_opens_a_separate_unsaved_recovery_composer() {
        corrupt_selected_recovery(SelectedCorruption::Summary);
    }
    #[test]
    fn corrupt_selected_message_opens_a_separate_unsaved_recovery_composer() {
        corrupt_selected_recovery(SelectedCorruption::Message);
    }
    #[test]
    fn corrupt_selected_generation_opens_a_separate_unsaved_recovery_composer() {
        corrupt_selected_recovery(SelectedCorruption::Generation);
    }
}
