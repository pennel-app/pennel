//! App-private, versioned conversation data. This store is never part of meeting sync/export.
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{MeetingAnswer, MeetingQuestionSelection, MeetingSourceAnchor, Passage, PennelError};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub(crate) const PAGE: usize = 50;
pub(crate) const MAX_RECOVERY_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_CACHED: usize = 24;
const MAX_SUMMARY_BYTES: usize = 256 * 1024;
const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const STORAGE_ERROR: &str = "Conversation storage is unavailable. Unsaved text is kept in this app; retry saving or copy it before quitting.";
pub(crate) fn invalid(message: &str) -> PennelError {
    PennelError::InvalidResponse {
        message: message.into(),
    }
}
pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub(crate) fn id() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Scope {
    Meeting {
        meeting_uid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        meeting_id: Option<String>,
    },
    Library,
}
impl Scope {
    pub(crate) fn uid(&self) -> Option<&str> {
        match self {
            Self::Meeting { meeting_uid, .. } => Some(meeting_uid),
            Self::Library => None,
        }
    }
    pub(crate) fn key(&self, library: &str) -> String {
        match self {
            Self::Meeting { meeting_uid, .. } => format!("{library}:meeting:{meeting_uid}"),
            Self::Library => format!("{library}:library"),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Summary {
    pub id: String,
    pub title: String,
    pub draft: String,
    pub draft_revision: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub active_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_state: Option<crate::automatic_titles::TitleState>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredSource {
    pub number: u32,
    pub passage: Passage,
    pub anchor: Option<MeetingSourceAnchor>,
    pub status: String,
    pub included_through_ms: u64,
    pub revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Evidence {
    pub request_id: String,
    pub meeting_uid: Option<String>,
    pub session_id: Option<String>,
    pub transcript_revision: u64,
    pub notes_revision: String,
    pub included_through_ms: u64,
    pub pending_transcription: u32,
    pub anchors: Vec<MeetingSourceAnchor>,
    pub sources: Vec<StoredSource>,
    pub selection: MeetingQuestionSelection,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Message {
    pub id: String,
    pub conversation_id: String,
    pub request_id: String,
    pub role: String,
    pub text: String,
    pub status: String,
    pub created_at_ms: i64,
    pub reply_to: Option<String>,
    pub answer: Option<MeetingAnswer>,
    pub evidence: Option<Evidence>,
    pub error: Option<String>,
    #[serde(skip)]
    pub position: i64,
}
#[derive(Debug, Clone)]
pub(crate) struct Cached {
    pub scope: Scope,
    pub scope_key: String,
    pub summary: Summary,
    pub messages: Vec<Message>,
    pub generation: u64,
    pub persisted: bool,
    pub dirty: bool,
}
impl Cached {
    pub(crate) fn new(scope: Scope, scope_key: String, title: String) -> Self {
        let timestamp = now();
        let automatic = title.trim().is_empty();
        let title = if automatic {
            crate::automatic_titles::dated_title("Conversation", chrono::Local::now())
        } else {
            title
        };
        let title_state = Some(crate::automatic_titles::TitleState::new(&title, automatic));
        Self {
            scope,
            scope_key,
            summary: Summary {
                id: id(),
                title,
                draft: String::new(),
                draft_revision: 0,
                created_at_ms: timestamp,
                updated_at_ms: timestamp,
                active_request_id: None,
                title_state,
            },
            messages: vec![],
            generation: 0,
            persisted: false,
            dirty: true,
        }
    }
    pub(crate) fn size(&self) -> usize {
        serde_json::to_vec(&self.summary).map_or(0, |v| v.len())
            + self
                .messages
                .iter()
                .map(|m| serde_json::to_vec(m).map_or(0, |v| v.len()))
                .sum::<usize>()
    }
    pub(crate) fn history(&self) -> Vec<crate::ChatTurn> {
        self.messages
            .iter()
            .filter(|m| m.role == "assistant" && m.status == "completed")
            .filter_map(|answer| {
                let question = self
                    .messages
                    .iter()
                    .find(|m| Some(&m.id) == answer.reply_to.as_ref() && m.role == "user")?;
                Some(crate::ChatTurn {
                    question: question.text.clone(),
                    answer: answer.text.clone(),
                })
            })
            .collect()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReadRequest {
    pub scope: Scope,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub messages_before: Option<String>,
    #[serde(default)]
    pub conversations_before: Option<String>,
    #[serde(default)]
    pub focus_message_id: Option<String>,
}
#[derive(Debug, Deserialize)]
pub(crate) struct UpdateRequest {
    pub scope: Scope,
    #[serde(default)]
    pub conversation_id: Option<String>,
    pub action: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub draft: Option<String>,
    #[serde(default)]
    pub expected_draft_revision: Option<u64>,
}
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BeginRequest {
    pub scope: Scope,
    pub conversation_id: String,
    pub request_id: String,
    pub question: String,
    #[serde(default)]
    pub expected_notes_revision: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}
#[derive(Debug, Deserialize)]
pub(crate) struct CancelRequest {
    pub scope: Scope,
    pub conversation_id: String,
    pub request_id: String,
}
#[derive(Debug, Deserialize)]
pub(crate) struct SourceRequest {
    pub scope: Scope,
    pub conversation_id: String,
    pub message_id: String,
    pub citation_number: u32,
}
#[derive(Debug, Serialize)]
pub(crate) struct SourceResolution {
    pub status: String,
    pub passage: Passage,
    pub anchor: Option<MeetingSourceAnchor>,
    pub current_meeting_id: Option<String>,
}
#[derive(Debug, Serialize)]
pub(crate) struct Snapshot {
    pub schema_version: u32,
    pub scope: Scope,
    pub current_meeting_id: Option<String>,
    pub storage_status: String,
    pub storage_error: Option<String>,
    pub selected_conversation_id: String,
    pub conversations: Vec<Summary>,
    pub conversation: Summary,
    pub messages: Vec<Message>,
    pub next_messages_cursor: Option<String>,
    pub next_conversations_cursor: Option<String>,
}

#[derive(Default)]
pub(crate) struct State {
    pub cancelled_admissions: HashSet<(String, String, String)>,
    pub legacy_aliases: HashMap<(String, String), String>,
    pub pending_cancelled_admissions: HashSet<(String, String, String)>,
    pub initialized: bool,
    pub cache: HashMap<String, Cached>,
    pub selected: HashMap<String, String>,
    pub dirty_selection: HashSet<String>,
    pub deleted: HashSet<String>,
    pub deleted_scopes: HashSet<String>,
    pub pending_deletes: HashMap<String, String>,
    pub used: HashMap<String, (String, String, String)>,
    pub active: HashMap<String, crate::conversation_engine::ActiveConversationRequest>,
    pub storage_error: Option<String>,
}
impl State {
    pub(crate) fn open(&mut self, data: &Path) -> Option<Store> {
        match Store::open(data).and_then(|store| {
            if !self.initialized {
                store.recover_pending()?;
            }
            Ok(store)
        }) {
            Ok(store) => {
                self.initialized = true;
                self.storage_error = None;
                Some(store)
            }
            Err(error) => {
                self.storage_error = Some(match error {
                    PennelError::InvalidResponse { message } => message,
                    _ => STORAGE_ERROR.into(),
                });
                None
            }
        }
    }
    pub(crate) fn persist(&mut self, store: Option<&mut Store>, conversation: &str) {
        let selected = self.cache.get(conversation).and_then(|doc| {
            if !self.dirty_selection.contains(&doc.scope_key) {
                return None;
            }
            let id = self.selected.get(&doc.scope_key)?;
            // Another recovered conversation may not exist in SQLite yet. Save this document
            // first and leave selection pending until its target can be committed atomically.
            (id == conversation || self.cache.get(id).is_some_and(|target| target.persisted))
                .then(|| (doc.scope_key.clone(), id.clone()))
        });
        let Some(doc) = self.cache.get_mut(conversation) else {
            return;
        };
        let result = store
            .ok_or_else(|| invalid(STORAGE_ERROR))
            .and_then(|store| store.save(doc, selected.as_ref()));
        match result {
            Ok(()) => {
                doc.persisted = true;
                doc.dirty = false;
                if selected.is_some() {
                    self.dirty_selection.remove(&doc.scope_key);
                }
                if doc.messages.len() > PAGE {
                    doc.messages.drain(..doc.messages.len() - PAGE);
                }
            }
            Err(_) => {
                doc.dirty = true;
                self.storage_error
                    .get_or_insert_with(|| STORAGE_ERROR.into());
            }
        }
    }
    pub(crate) fn room(&mut self, protected: &str, extra: usize) -> Result<(), PennelError> {
        while self.cache.len() + usize::from(!self.cache.contains_key(protected)) > MAX_CACHED
            || self
                .cache
                .values()
                .map(Cached::size)
                .sum::<usize>()
                .saturating_add(extra)
                > MAX_RECOVERY_BYTES
        {
            let evict = self
                .cache
                .iter()
                .filter(|(id, doc)| {
                    id.as_str() != protected
                        && !doc.dirty
                        && doc.summary.active_request_id.is_none()
                })
                .min_by_key(|(_, doc)| doc.summary.updated_at_ms)
                .map(|(id, _)| id.clone());
            let Some(evict) = evict else {
                return Err(invalid(
                    "The unsaved conversation recovery buffer is full. Save or copy existing text before asking again.",
                ));
            };
            self.cache.remove(&evict);
        }
        Ok(())
    }
}

pub(crate) struct Store {
    connection: Connection,
}
struct ConversationHeader {
    scope_key: String,
    scope: Scope,
    summary: Summary,
    generation: u64,
}
struct MessageRow {
    id: String,
    conversation_id: String,
    status: String,
    position: i64,
    body: String,
}
impl Store {
    pub(crate) fn open(data: &Path) -> Result<Self, PennelError> {
        let path = data.join("conversations.sqlite");
        if std::fs::symlink_metadata(&path)
            .is_ok_and(|m| m.file_type().is_symlink() || !m.is_file())
        {
            return Err(invalid(STORAGE_ERROR));
        }
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(options.open(&path).map_err(PennelError::storage)?);
        let connection = Connection::open(&path).map_err(PennelError::storage)?;
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(PennelError::storage)?;
        if version > 1 {
            return Err(invalid(
                "The conversation store requires a newer Pennel version. Its original data was kept.",
            ));
        }
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;PRAGMA synchronous=FULL;PRAGMA busy_timeout=2500;",
            )
            .map_err(PennelError::storage)?;
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE conversations(id TEXT PRIMARY KEY,scope TEXT NOT NULL,scope_json TEXT NOT NULL,summary TEXT NOT NULL,generation INTEGER NOT NULL,updated_at_ms INTEGER NOT NULL,active_request_id TEXT);
            CREATE INDEX conversations_scope ON conversations(scope,updated_at_ms DESC,id DESC);
            CREATE TABLE messages(id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,position INTEGER NOT NULL,body TEXT NOT NULL,status TEXT NOT NULL,UNIQUE(conversation_id,position));
            CREATE INDEX messages_page ON messages(conversation_id,position DESC);
            CREATE TABLE selections(scope TEXT PRIMARY KEY,conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL);
            CREATE TABLE used_requests(request_id TEXT PRIMARY KEY,scope TEXT NOT NULL,conversation_id TEXT NOT NULL,question_hash TEXT NOT NULL);
            PRAGMA user_version=1;COMMIT;").map_err(PennelError::storage)?;
        }
        connection.execute_batch("CREATE TABLE IF NOT EXISTS cancelled_admissions(scope TEXT NOT NULL,conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,request_id TEXT NOT NULL,PRIMARY KEY(scope,conversation_id,request_id));").map_err(PennelError::storage)?;
        Ok(Self { connection })
    }
    fn summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, Option<String>, String)> {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    }
    fn summary(
        text: String,
        active: Option<String>,
        expected_id: &str,
    ) -> Result<Summary, PennelError> {
        let mut summary: Summary =
            serde_json::from_str(&text).map_err(|_| Self::metadata_error())?;
        if summary.id != expected_id || expected_id.is_empty() {
            return Err(Self::metadata_error());
        }
        summary.active_request_id = active;
        Ok(summary)
    }
    fn metadata_error() -> PennelError {
        invalid("The saved conversation metadata could not be read. Its original data was kept.")
    }
    fn message_error() -> PennelError {
        invalid("A saved conversation message could not be read. Its original data was kept.")
    }
    fn header(
        connection: &Connection,
        id: &str,
    ) -> Result<Option<ConversationHeader>, PennelError> {
        let row:Option<(String,String,String,i64,Option<String>)>=connection.query_row("SELECT scope,scope_json,CASE WHEN octet_length(summary)<=262144 THEN summary END,generation,active_request_id FROM conversations WHERE id=?1",[id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).optional().map_err(PennelError::storage)?;
        let Some((key, scope, summary, generation, active)) = row else {
            return Ok(None);
        };
        let scope: Scope = serde_json::from_str(&scope)
            .map_err(|_| invalid("The saved conversation scope could not be read."))?;
        let suffix = match &scope {
            Scope::Library => ":library".into(),
            Scope::Meeting { meeting_uid, .. } => format!(":meeting:{meeting_uid}"),
        };
        if !key.ends_with(&suffix) {
            return Err(Self::metadata_error());
        }
        Ok(Some(ConversationHeader {
            scope_key: key,
            scope,
            summary: Self::summary(summary, active, id)?,
            generation: u64::try_from(generation).map_err(|_| Self::metadata_error())?,
        }))
    }
    fn message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRow> {
        Ok(MessageRow {
            id: row.get(0)?,
            conversation_id: row.get(1)?,
            status: row.get(2)?,
            position: row.get(3)?,
            body: row.get(4)?,
        })
    }
    fn raw_message(
        connection: &Connection,
        conversation: &str,
        id: &str,
    ) -> Result<Option<MessageRow>, PennelError> {
        connection.query_row("SELECT id,conversation_id,status,position,CASE WHEN octet_length(body)<=4194304 THEN body END FROM messages WHERE conversation_id=?1 AND id=?2",params![conversation,id],Self::message_row).optional().map_err(PennelError::storage)
    }
    fn check_message_keys(
        message: &Message,
        id: &str,
        conversation: &str,
        status: &str,
        position: i64,
    ) -> Result<(), PennelError> {
        if id.is_empty()
            || message.id != id
            || message.conversation_id != conversation
            || position < 0
            || message.request_id.is_empty()
            || message.request_id.len() > 256
            || message.request_id.chars().any(char::is_control)
            || !["user", "assistant"].contains(&message.role.as_str())
            || !["completed", "pending", "cancelled", "interrupted", "failed"].contains(&status)
            || (message.status != status
                && !(status == "interrupted" && message.status == "pending"))
            || (message.role == "user" && message.reply_to.is_some())
            || (message.role == "assistant"
                && message.reply_to.as_ref().is_none_or(|id| id.is_empty()))
            || message
                .evidence
                .as_ref()
                .is_some_and(|evidence| evidence.request_id != message.request_id)
        {
            return Err(Self::message_error());
        }
        Ok(())
    }
    fn decode_message_base(row: MessageRow) -> Result<Message, PennelError> {
        let mut message: Message =
            serde_json::from_str(&row.body).map_err(|_| Self::message_error())?;
        Self::check_message_keys(
            &message,
            &row.id,
            &row.conversation_id,
            &row.status,
            row.position,
        )?;
        message.status = row.status;
        message.position = row.position;
        if message.status == "interrupted" {
            message.error = Some("The answer was interrupted before it finished.".into());
        }
        Ok(message)
    }
    fn check_request_binding(
        connection: &Connection,
        key: &str,
        message: &Message,
    ) -> Result<(), PennelError> {
        let binding: Option<(String, String, String)> = connection
            .query_row(
                "SELECT scope,conversation_id,question_hash FROM used_requests WHERE request_id=?1",
                [&message.request_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(PennelError::storage)?;
        let Some((scope, conversation, question_hash)) = binding else {
            return Err(Self::message_error());
        };
        if scope != key || conversation != message.conversation_id {
            return Err(Self::message_error());
        }
        if message.role == "user" {
            if crate::evidence_revision(&message.text) != question_hash {
                return Err(Self::message_error());
            }
        } else {
            let reply = message
                .reply_to
                .as_deref()
                .ok_or_else(Self::message_error)?;
            let user = Self::raw_message(connection, &message.conversation_id, reply)?
                .ok_or_else(Self::message_error)?;
            let user = Self::decode_message_base(user)?;
            if user.role != "user"
                || user.request_id != message.request_id
                || user.position >= message.position
                || crate::evidence_revision(&user.text) != question_hash
            {
                return Err(Self::message_error());
            }
        }
        Ok(())
    }
    fn decode_message(
        connection: &Connection,
        key: &str,
        row: MessageRow,
    ) -> Result<Message, PennelError> {
        let message = Self::decode_message_base(row)?;
        Self::check_request_binding(connection, key, &message)?;
        Ok(message)
    }
    pub(crate) fn recover_pending(&self) -> Result<(), PennelError> {
        self.connection.execute_batch("BEGIN IMMEDIATE;UPDATE messages SET status='interrupted' WHERE status='pending';UPDATE conversations SET active_request_id=NULL;COMMIT;").map_err(PennelError::storage)
    }
    pub(crate) fn selected(&self, key: &str) -> Result<Option<String>, PennelError> {
        self.connection
            .query_row(
                "SELECT conversation_id FROM selections WHERE scope=?1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .map(|v| v.flatten())
            .map_err(PennelError::storage)
    }
    pub(crate) fn list(
        &self,
        key: &str,
        cursor: Option<&str>,
    ) -> Result<Vec<Summary>, PennelError> {
        let cursor = cursor.map(parse_summary_cursor).transpose()?;
        let (time, id) = cursor.unwrap_or((i64::MAX, String::from("~")));
        let mut statement=self.connection.prepare("SELECT CASE WHEN octet_length(summary)<=262144 THEN summary END,active_request_id,id FROM conversations WHERE scope=?1 AND (updated_at_ms<?2 OR (updated_at_ms=?2 AND id<?3)) ORDER BY updated_at_ms DESC,id DESC LIMIT 51").map_err(PennelError::storage)?;
        statement
            .query_map(params![key, time, id], Self::summary_row)
            .map_err(PennelError::storage)?
            .map(|row| {
                let (text, active, id) = row.map_err(PennelError::storage)?;
                Self::summary(text, active, &id)
            })
            .collect()
    }
    pub(crate) fn load(&self, key: &str, id: &str) -> Result<Option<Cached>, PennelError> {
        let Some(header) = Self::header(&self.connection, id)? else {
            return Ok(None);
        };
        if header.scope_key != key {
            return Ok(None);
        }
        Ok(Some(Cached {
            scope: header.scope,
            scope_key: header.scope_key,
            summary: header.summary,
            messages: self.page(id, None, None)?.0,
            generation: header.generation,
            persisted: true,
            dirty: false,
        }))
    }
    pub(crate) fn page(
        &self,
        conversation: &str,
        before: Option<&str>,
        focus: Option<&str>,
    ) -> Result<(Vec<Message>, Option<String>), PennelError> {
        let header = Self::header(&self.connection, conversation)?.ok_or(PennelError::NotFound)?;
        let upper = if let Some(focus) = focus {
            self.connection
                .query_row(
                    "SELECT position+1 FROM messages WHERE conversation_id=?1 AND id=?2",
                    params![conversation, focus],
                    |r| r.get::<_, i64>(0),
                )
                .optional()
                .map_err(PennelError::storage)?
                .ok_or(PennelError::NotFound)?
        } else {
            before
                .map(|v| {
                    v.parse::<i64>()
                        .map_err(|_| invalid("The conversation page cursor is invalid."))
                })
                .transpose()?
                .unwrap_or(i64::MAX)
        };
        let mut statement=self.connection.prepare("SELECT id,conversation_id,status,position,CASE WHEN octet_length(body)<=4194304 THEN body END FROM messages WHERE conversation_id=?1 AND position<?2 ORDER BY position DESC LIMIT 51").map_err(PennelError::storage)?;
        let rows = statement
            .query_map(params![conversation, upper], Self::message_row)
            .map_err(PennelError::storage)?;
        let mut messages = vec![];
        let mut bytes = 0;
        for row in rows {
            let row = row.map_err(PennelError::storage)?;
            if messages.len() >= PAGE
                || (!messages.is_empty() && bytes + row.body.len() > 4 * 1024 * 1024)
            {
                break;
            }
            bytes += row.body.len();
            messages.push(Self::decode_message(
                &self.connection,
                &header.scope_key,
                row,
            )?);
        }
        let next=messages.last().map(|m|m.position).map(|position|self.connection.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND position<?2)",params![conversation,position],|r|r.get::<_,bool>(0)).map(|older|older.then(||position.to_string()))).transpose().map_err(PennelError::storage)?.flatten();
        messages.reverse();
        Ok((messages, next))
    }
    pub(crate) fn message(
        &self,
        conversation: &str,
        id: &str,
    ) -> Result<Option<Message>, PennelError> {
        let header = Self::header(&self.connection, conversation)?.ok_or(PennelError::NotFound)?;
        Self::raw_message(&self.connection, conversation, id)?
            .map(|row| Self::decode_message(&self.connection, &header.scope_key, row))
            .transpose()
    }

    pub(crate) fn initial_messages(&self, conversation: &str) -> Result<Vec<Message>, PennelError> {
        let header = Self::header(&self.connection, conversation)?.ok_or(PennelError::NotFound)?;
        let mut statement = self.connection.prepare("SELECT id,conversation_id,status,position,CASE WHEN octet_length(body)<=4194304 THEN body END FROM messages WHERE conversation_id=?1 ORDER BY position LIMIT 2").map_err(PennelError::storage)?;
        statement
            .query_map([conversation], Self::message_row)
            .map_err(PennelError::storage)?
            .map(|row| {
                Self::decode_message(
                    &self.connection,
                    &header.scope_key,
                    row.map_err(PennelError::storage)?,
                )
            })
            .collect()
    }

    /// Only title fields change; another process's draft and request metadata remain authoritative.
    pub(crate) fn update_title(
        &mut self,
        key: &str,
        expected: &Summary,
        title: &str,
        state: &crate::automatic_titles::TitleState,
    ) -> Result<Option<Summary>, PennelError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(PennelError::storage)?;
        let Some(header) = Self::header(&transaction, &expected.id)? else {
            return Ok(None);
        };
        if header.scope_key != key
            || header.summary.title != expected.title
            || header.summary.title_state != expected.title_state
        {
            return Ok(None);
        }
        let mut summary = header.summary;
        summary.title = title.into();
        summary.title_state = Some(state.clone());
        summary.updated_at_ms = now();
        let serialized = serde_json::to_string(&summary).map_err(PennelError::storage)?;
        transaction
            .execute(
                "UPDATE conversations SET summary=?1,updated_at_ms=?2 WHERE id=?3 AND scope=?4",
                params![serialized, summary.updated_at_ms, summary.id, key],
            )
            .map_err(PennelError::storage)?;
        transaction.commit().map_err(PennelError::storage)?;
        Ok(Some(summary))
    }
    pub(crate) fn used(
        &self,
        request: &str,
    ) -> Result<Option<(String, String, String)>, PennelError> {
        self.connection
            .query_row(
                "SELECT scope,conversation_id,question_hash FROM used_requests WHERE request_id=?1",
                [request],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(PennelError::storage)
    }

    pub(crate) fn admission_cancelled(
        &self,
        key: &str,
        conversation: &str,
        request: &str,
    ) -> Result<bool, PennelError> {
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM cancelled_admissions WHERE scope=?1 AND conversation_id=?2 AND request_id=?3)",params![key,conversation,request],|row|row.get(0)).map_err(PennelError::storage)
    }
    pub(crate) fn cancel_admission(
        &self,
        key: &str,
        conversation: &str,
        request: &str,
    ) -> Result<(), PennelError> {
        self.connection.execute("INSERT OR IGNORE INTO cancelled_admissions(scope,conversation_id,request_id) VALUES(?1,?2,?3)",params![key,conversation,request]).map(|_|()).map_err(PennelError::storage)
    }
    pub(crate) fn save(
        &mut self,
        doc: &Cached,
        selected: Option<&(String, String)>,
    ) -> Result<(), PennelError> {
        if doc.summary.id.is_empty() {
            return Err(Self::metadata_error());
        }
        let summary = serde_json::to_string(&doc.summary).map_err(PennelError::storage)?;
        if summary.len() > MAX_SUMMARY_BYTES {
            return Err(invalid(STORAGE_ERROR));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(PennelError::storage)?;
        if let Some(header) = Self::header(&transaction, &doc.summary.id)?
            && (header.scope_key != doc.scope_key || header.scope != doc.scope)
        {
            return Err(Self::metadata_error());
        }
        let generation = i64::try_from(doc.generation)
            .map_err(|_| invalid("The conversation generation is invalid."))?;
        if !doc.persisted {
            transaction.execute("INSERT OR IGNORE INTO conversations(id,scope,scope_json,summary,generation,updated_at_ms,active_request_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![doc.summary.id,doc.scope_key,serde_json::to_string(&doc.scope).map_err(PennelError::storage)?,summary,generation,doc.summary.updated_at_ms,doc.summary.active_request_id]).map_err(PennelError::storage)?;
        }
        let changed=transaction.execute("UPDATE conversations SET summary=?1,updated_at_ms=?2,active_request_id=?3 WHERE id=?4 AND scope=?5 AND generation=?6",params![summary,doc.summary.updated_at_ms,doc.summary.active_request_id,doc.summary.id,doc.scope_key,generation]).map_err(PennelError::storage)?;
        if changed != 1 {
            return Err(PennelError::NotFound);
        }
        let mut user_requests = HashMap::new();
        for message in &doc.messages {
            Self::check_message_keys(
                message,
                &message.id,
                &doc.summary.id,
                &message.status,
                message.position,
            )?;
            if message.role == "user"
                && user_requests
                    .insert(&message.request_id, &message.id)
                    .is_some_and(|previous| previous != &message.id)
            {
                return Err(Self::message_error());
            }
            if let Some(row) = Self::raw_message(&transaction, &doc.summary.id, &message.id)? {
                let stored = Self::decode_message(&transaction, &doc.scope_key, row)?;
                if stored.role != message.role
                    || stored.request_id != message.request_id
                    || stored.reply_to != message.reply_to
                    || stored.position != message.position
                {
                    return Err(Self::message_error());
                }
            }
            let body = serde_json::to_string(message).map_err(PennelError::storage)?;
            if body.len() > MAX_MESSAGE_BYTES {
                return Err(invalid(STORAGE_ERROR));
            }
            let changed=transaction.execute("INSERT INTO messages(id,conversation_id,position,body,status) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET body=excluded.body,status=excluded.status WHERE messages.conversation_id=excluded.conversation_id AND messages.position=excluded.position AND (messages.body!=excluded.body OR messages.status!=excluded.status)",params![message.id,doc.summary.id,message.position,body,message.status]).map_err(PennelError::storage)?;
            if changed == 0 {
                let exact:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2 AND position=?3 AND body=?4 AND status=?5)",params![message.id,doc.summary.id,message.position,body,message.status],|row|row.get(0)).map_err(PennelError::storage)?;
                if !exact {
                    return Err(PennelError::NotFound);
                }
            }
            if message.role == "user" {
                let retained=transaction.execute("INSERT INTO used_requests(request_id,scope,conversation_id,question_hash) VALUES(?1,?2,?3,?4) ON CONFLICT(request_id) DO UPDATE SET question_hash=used_requests.question_hash WHERE used_requests.scope=excluded.scope AND used_requests.conversation_id=excluded.conversation_id AND used_requests.question_hash=excluded.question_hash",params![message.request_id,doc.scope_key,doc.summary.id,crate::evidence_revision(&message.text)]).map_err(PennelError::storage)?;
                if retained != 1 {
                    return Err(PennelError::NotFound);
                }
            }
        }
        for message in &doc.messages {
            Self::check_request_binding(&transaction, &doc.scope_key, message)?;
        }
        if let Some((key, id)) = selected {
            let scoped: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM conversations WHERE scope=?1 AND id=?2)",
                    params![key, id],
                    |row| row.get(0),
                )
                .map_err(PennelError::storage)?;
            if !scoped {
                return Err(PennelError::NotFound);
            }
            transaction.execute("INSERT INTO selections(scope,conversation_id) VALUES(?1,?2) ON CONFLICT(scope) DO UPDATE SET conversation_id=excluded.conversation_id",params![key,id]).map_err(PennelError::storage)?;
        }
        transaction.commit().map_err(PennelError::storage)
    }
    pub(crate) fn delete(&mut self, key: &str, id: &str) -> Result<(), PennelError> {
        self.connection
            .execute(
                "DELETE FROM conversations WHERE scope=?1 AND id=?2",
                params![key, id],
            )
            .map(|_| ())
            .map_err(PennelError::storage)
    }
    pub(crate) fn delete_scope(&mut self, key: &str) -> Result<(), PennelError> {
        self.connection
            .execute("DELETE FROM conversations WHERE scope=?1", [key])
            .map(|_| ())
            .map_err(PennelError::storage)
    }
}
pub(crate) fn summary_cursor(summary: &Summary) -> String {
    format!("{}:{}", summary.updated_at_ms, summary.id)
}
pub(crate) fn parse_summary_cursor(cursor: &str) -> Result<(i64, String), PennelError> {
    let (time, id) = cursor
        .split_once(':')
        .ok_or_else(|| invalid("The conversation page cursor is invalid."))?;
    Ok((
        time.parse()
            .map_err(|_| invalid("The conversation page cursor is invalid."))?,
        id.into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation() -> Cached {
        let scope = Scope::Library;
        let key = scope.key("isolated-test-library");
        Cached::new(scope, key, String::new())
    }

    fn question(conversation: &str, request: &str, text: &str, position: i64) -> Message {
        Message {
            id: id(),
            conversation_id: conversation.into(),
            request_id: request.into(),
            role: "user".into(),
            text: text.into(),
            status: "completed".into(),
            created_at_ms: now(),
            reply_to: None,
            answer: None,
            evidence: None,
            error: None,
            position,
        }
    }

    #[test]
    fn interrupted_save_rolls_back_metadata_and_keeps_existing_messages_readable() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(directory.path()).unwrap();
        let mut doc = conversation();
        doc.summary.title = "Saved title".into();
        store
            .save(&doc, Some(&(doc.scope_key.clone(), doc.summary.id.clone())))
            .unwrap();
        doc.persisted = true;
        doc.summary.title = "Uncommitted title".into();
        doc.messages.push(question(
            &doc.summary.id,
            &id(),
            "Retain this question for retry",
            0,
        ));
        let fault = Connection::open(directory.path().join("conversations.sqlite")).unwrap();
        fault.execute_batch("CREATE TRIGGER fail_message BEFORE INSERT ON messages BEGIN SELECT RAISE(FAIL,'injected message persistence failure'); END;").unwrap();
        assert!(store.save(&doc, None).is_err());
        let retained = store
            .load(&doc.scope_key, &doc.summary.id)
            .unwrap()
            .unwrap();
        assert_eq!(retained.summary.title, "Saved title");
        assert!(retained.messages.is_empty());
        fault.execute_batch("DROP TRIGGER fail_message;").unwrap();
        store.save(&doc, None).unwrap();
        let retried = store
            .load(&doc.scope_key, &doc.summary.id)
            .unwrap()
            .unwrap();
        assert_eq!(retried.summary.title, "Uncommitted title");
        assert_eq!(retried.messages[0].text, "Retain this question for retry");
    }

    #[test]
    fn pending_recovery_runs_once_per_engine_state_and_preserves_live_jobs_when_reopened() {
        let directory = tempfile::tempdir().unwrap();
        let mut runtime = State::default();
        let mut store = runtime.open(directory.path()).unwrap();
        let mut doc = conversation();
        let request = id();
        let asked = question(&doc.summary.id, &request, "A pending question", 0);
        let response = Message {
            id: id(),
            conversation_id: doc.summary.id.clone(),
            request_id: request.clone(),
            role: "assistant".into(),
            text: String::new(),
            status: "pending".into(),
            created_at_ms: now(),
            reply_to: Some(asked.id.clone()),
            answer: None,
            evidence: None,
            error: None,
            position: 1,
        };
        doc.messages = vec![asked, response];
        doc.summary.active_request_id = Some(request.clone());
        store.save(&doc, None).unwrap();
        drop(store);
        let reopened_in_same_runtime = runtime.open(directory.path()).unwrap();
        let current = reopened_in_same_runtime
            .load(&doc.scope_key, &doc.summary.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            current.summary.active_request_id.as_deref(),
            Some(request.as_str())
        );
        assert_eq!(current.messages[1].status, "pending");
        drop(reopened_in_same_runtime);
        let mut next_runtime = State::default();
        let restarted = next_runtime.open(directory.path()).unwrap();
        let interrupted = restarted
            .load(&doc.scope_key, &doc.summary.id)
            .unwrap()
            .unwrap();
        assert!(interrupted.summary.active_request_id.is_none());
        assert_eq!(interrupted.messages[0].text, "A pending question");
        assert_eq!(interrupted.messages[1].status, "interrupted");
        assert!(interrupted.messages[1].answer.is_none());
        let single = restarted
            .message(&doc.summary.id, &interrupted.messages[1].id)
            .unwrap()
            .unwrap();
        assert_eq!(
            single.error.as_deref(),
            Some("The answer was interrupted before it finished.")
        );
    }

    #[test]
    fn request_tombstones_survive_deletion_and_conflicting_reuse_rolls_back_new_conversations() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(directory.path()).unwrap();
        let mut original = conversation();
        let request = id();
        original.messages.push(question(
            &original.summary.id,
            &request,
            "The first question",
            0,
        ));
        store.save(&original, None).unwrap();
        store.save(&original, None).unwrap();
        store
            .delete(&original.scope_key, &original.summary.id)
            .unwrap();
        let binding = store.used(&request).unwrap().unwrap();
        assert_eq!(binding.1, original.summary.id);
        original.persisted = true;
        assert!(store.save(&original, None).is_err());
        assert!(
            store
                .load(&original.scope_key, &original.summary.id)
                .unwrap()
                .is_none()
        );
        let mut conflicting = conversation();
        conflicting.messages.push(question(
            &conflicting.summary.id,
            &request,
            "A different question",
            0,
        ));
        assert!(store.save(&conflicting, None).is_err());
        assert!(
            store
                .load(&conflicting.scope_key, &conflicting.summary.id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn message_pages_and_old_answer_focus_are_bounded_without_losing_history() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(directory.path()).unwrap();
        let mut doc = conversation();
        for position in 0..120 {
            doc.messages.push(question(
                &doc.summary.id,
                &id(),
                &format!("Question {position}"),
                position,
            ));
        }
        let focused = doc.messages[10].id.clone();
        store.save(&doc, None).unwrap();
        let (latest, before) = store.page(&doc.summary.id, None, None).unwrap();
        assert_eq!(latest.len(), 50);
        assert_eq!(latest[0].text, "Question 70");
        let (middle, before) = store
            .page(&doc.summary.id, before.as_deref(), None)
            .unwrap();
        assert_eq!(middle.len(), 50);
        assert_eq!(middle[0].text, "Question 20");
        let (first, before) = store
            .page(&doc.summary.id, before.as_deref(), None)
            .unwrap();
        assert_eq!(first.len(), 20);
        assert_eq!(first[0].text, "Question 0");
        assert!(before.is_none());
        let (returned, _) = store.page(&doc.summary.id, None, Some(&focused)).unwrap();
        assert!(returned.len() <= 50);
        assert_eq!(returned.last().unwrap().id, focused);
    }

    #[test]
    fn saving_recovery_conversations_before_the_selected_new_one_keeps_selection_pending() {
        let directory = tempfile::tempdir().unwrap();
        let mut runtime = State::default();
        let mut store = runtime.open(directory.path()).unwrap();
        let first = conversation();
        let second = conversation();
        let key = first.scope_key.clone();
        let first_id = first.summary.id.clone();
        let second_id = second.summary.id.clone();
        runtime.cache.insert(first_id.clone(), first);
        runtime.cache.insert(second_id.clone(), second);
        runtime.selected.insert(key.clone(), second_id.clone());
        runtime.dirty_selection.insert(key.clone());
        runtime.persist(Some(&mut store), &first_id);
        assert!(!runtime.cache[&first_id].dirty);
        assert!(runtime.dirty_selection.contains(&key));
        assert!(store.selected(&key).unwrap().is_none());
        runtime.persist(Some(&mut store), &second_id);
        assert!(!runtime.cache[&second_id].dirty);
        assert!(!runtime.dirty_selection.contains(&key));
        assert_eq!(
            store.selected(&key).unwrap().as_deref(),
            Some(second_id.as_str())
        );
    }

    #[test]
    fn corrupt_summary_identity_cannot_redirect_writes_to_another_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(directory.path()).unwrap();
        let mut first = conversation();
        first.summary.title = "Conversation A".into();
        let mut second = conversation();
        second.summary.title = "Conversation B".into();
        second.summary.draft = "B's retained draft".into();
        store.save(&first, None).unwrap();
        store.save(&second, None).unwrap();
        let fault = Connection::open(directory.path().join("conversations.sqlite")).unwrap();
        let mut corrupt = first.summary.clone();
        corrupt.id = second.summary.id.clone();
        let corrupt_bytes = serde_json::to_string(&corrupt).unwrap();
        fault
            .execute(
                "UPDATE conversations SET summary=?1 WHERE id=?2",
                params![corrupt_bytes, first.summary.id],
            )
            .unwrap();
        assert!(store.load(&first.scope_key, &first.summary.id).is_err());
        assert!(store.list(&first.scope_key, None).is_err());
        first.persisted = true;
        first.summary.title = "Attempted write through A".into();
        assert!(store.save(&first, None).is_err());
        let kept = store
            .load(&second.scope_key, &second.summary.id)
            .unwrap()
            .unwrap();
        assert_eq!(kept.summary.title, "Conversation B");
        assert_eq!(kept.summary.draft, "B's retained draft");
        let after: String = fault
            .query_row(
                "SELECT summary FROM conversations WHERE id=?1",
                [&first.summary.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after, corrupt_bytes);
    }

    #[test]
    fn corrupted_message_identity_is_rejected_on_pages_single_reads_and_cached_saves() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(directory.path()).unwrap();
        let mut doc = conversation();
        doc.summary.title = "Retained conversation".into();
        let request = id();
        let asked = question(&doc.summary.id, &request, "Retained question", 0);
        let response = Message {
            id: id(),
            conversation_id: doc.summary.id.clone(),
            request_id: request,
            role: "assistant".into(),
            text: String::new(),
            status: "pending".into(),
            created_at_ms: now(),
            reply_to: Some(asked.id.clone()),
            answer: None,
            evidence: None,
            error: None,
            position: 1,
        };
        let response_id = response.id.clone();
        let original = serde_json::to_value(&response).unwrap();
        doc.messages = vec![asked, response];
        store.save(&doc, None).unwrap();
        doc.persisted = true;
        let fault = Connection::open(directory.path().join("conversations.sqlite")).unwrap();
        for (field, value) in [
            ("id", id()),
            ("conversation_id", id()),
            ("role", "user".into()),
            ("request_id", id()),
        ] {
            let mut corrupted = original.clone();
            corrupted[field] = serde_json::json!(value);
            let bytes = corrupted.to_string();
            fault
                .execute(
                    "UPDATE messages SET body=?1 WHERE id=?2",
                    params![bytes, response_id],
                )
                .unwrap();
            assert!(
                store.page(&doc.summary.id, None, None).is_err(),
                "field {field}"
            );
            assert!(
                store.message(&doc.summary.id, &response_id).is_err(),
                "field {field}"
            );
            doc.summary.title = "A cached save must not repair corruption".into();
            assert!(store.save(&doc, None).is_err(), "field {field}");
            assert_eq!(
                store.list(&doc.scope_key, None).unwrap()[0].title,
                "Retained conversation"
            );
            let after: String = fault
                .query_row(
                    "SELECT body FROM messages WHERE id=?1",
                    [&response_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(after, bytes);
            fault
                .execute(
                    "UPDATE messages SET body=?1 WHERE id=?2",
                    params![original.to_string(), response_id],
                )
                .unwrap();
        }
        assert_eq!(store.page(&doc.summary.id, None, None).unwrap().0.len(), 2);
    }
}
