//! Claude access relay: a long-poll channel through which Pennel Cloud hands this device the
//! questions Claude asks about the user's meetings. See `docs/claude-access.md`.

use std::sync::Arc;

use serde_json::{Value, json};

use super::account::Session;
use super::http::{agent, error_from, send_json};
use crate::error::PennelError;

/// The server holds a poll for up to 25 s; this leaves room for a slow network.
const POLL_TIMEOUT_SECS: u64 = 40;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RelayRequest {
    pub id: String,
    pub tool: String,
    pub arguments_json: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RelayAnswer {
    Text { text: String },
    Failed { code: String, message: String },
}

pub struct RelayClient {
    session: Arc<Session>,
    http: ureq::Agent,
}

impl RelayClient {
    pub fn new(session: Arc<Session>) -> Self {
        Self {
            session,
            http: agent(POLL_TIMEOUT_SECS),
        }
    }

    /// A 401 gets one forced refresh, in case the access token was revoked before it expired.
    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Value), PennelError> {
        let url = format!("{}{path}", self.session.base_url());
        let epoch = self.session.generation();
        let token = self.session.access_token()?;
        let mut response = send_json(&self.http, method, &url, Some(&token), body)?;
        if response.0 == 401 {
            let token = self.session.refreshed_token()?;
            response = send_json(&self.http, method, &url, Some(&token), body)?;
        }
        if epoch != self.session.generation() || !self.session.signed_in() {
            return Err(PennelError::Cancelled);
        }
        Ok(response)
    }

    fn post(&self, path: &str, body: &Value) -> Result<(u16, Value), PennelError> {
        self.request("POST", path, Some(body))
    }

    pub fn poll(&self, client: &str) -> Result<Vec<RelayRequest>, PennelError> {
        self.poll_body(&json!({ "client": client }))
    }

    pub fn poll_with_capabilities(
        &self,
        client: &str,
        capabilities: &[String],
    ) -> Result<Vec<RelayRequest>, PennelError> {
        self.poll_body(&json!({"client": client, "capabilities": capabilities}))
    }

    pub fn writing_style_source(&self, enabled: Option<bool>) -> Result<bool, PennelError> {
        let method = match enabled {
            None => "GET",
            Some(true) => "PUT",
            Some(false) => "DELETE",
        };
        let body = enabled.map(|_| json!({}));
        let (status, response) = self.request(method, "/v1/writing-style/source", body.as_ref())?;
        if status != 200 {
            return Err(error_from(status, &response));
        }
        response
            .get("designated")
            .and_then(Value::as_bool)
            .ok_or_else(|| PennelError::InvalidResponse {
                message: "unexpected writing profile source response".into(),
            })
    }

    fn poll_body(&self, request: &Value) -> Result<Vec<RelayRequest>, PennelError> {
        let (status, body) = self.post("/v1/relay/poll", request)?;
        if status != 200 {
            return Err(error_from(status, &body));
        }
        let requests = body
            .get("requests")
            .and_then(Value::as_array)
            .ok_or_else(|| PennelError::InvalidResponse {
                message: "unexpected relay poll response".into(),
            })?;
        Ok(requests
            .iter()
            .filter_map(|request| {
                Some(RelayRequest {
                    id: request.get("id")?.as_str()?.to_owned(),
                    tool: request.get("tool")?.as_str()?.to_owned(),
                    arguments_json: request
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({}))
                        .to_string(),
                    timeout_ms: request
                        .get("timeout_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or(20_000),
                })
            })
            .collect())
    }

    pub fn respond(&self, id: &str, answer: &RelayAnswer) -> Result<(), PennelError> {
        let body = match answer {
            RelayAnswer::Text { text } => json!({ "id": id, "result": { "text": text } }),
            RelayAnswer::Failed { code, message } => {
                json!({ "id": id, "error": { "code": code, "message": message } })
            }
        };
        let (status, response) = self.post("/v1/relay/respond", &body)?;
        // 404: Claude stopped waiting; there is nobody left to answer.
        if (200..300).contains(&status) || status == 404 {
            return Ok(());
        }
        Err(error_from(status, &response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::REFRESH_ACCOUNT;
    use crate::cloud::SecretStore;
    use crate::cloud::tests::MemorySecrets;
    use mockito::Matcher;

    fn client(server: &mockito::ServerGuard) -> RelayClient {
        let secrets = Arc::new(MemorySecrets::default());
        secrets.set(REFRESH_ACCOUNT.into(), Some("ref".into()));
        RelayClient::new(Arc::new(Session::new(server.url(), secrets)))
    }

    fn refresh(server: &mut mockito::ServerGuard, access: &str, rotated: &str) -> mockito::Mock {
        server
            .mock("POST", "/v1/auth/refresh")
            .with_body(
                json!({"access_token": access, "refresh_token": rotated, "expires_in": 900})
                    .to_string(),
            )
            .create()
    }

    #[test]
    fn poll_parses_requests_with_bearer() {
        let mut server = mockito::Server::new();
        let _refresh = refresh(&mut server, "acc", "ref2");
        let poll = server
            .mock("POST", "/v1/relay/poll")
            .match_header("authorization", "Bearer acc")
            .match_body(Matcher::Json(json!({"client": "pennel-desktop/1.0"})))
            .with_body(r#"{"requests":[{"id":"r1","tool":"search_meetings","arguments":{"query":"budget"},"timeout_ms":20000},{"tool":"no id"}]}"#)
            .create();
        let requests = client(&server).poll("pennel-desktop/1.0").unwrap();
        poll.assert();
        assert_eq!(
            requests,
            vec![RelayRequest {
                id: "r1".into(),
                tool: "search_meetings".into(),
                arguments_json: r#"{"query":"budget"}"#.into(),
                timeout_ms: 20000,
            }]
        );
    }

    #[test]
    fn empty_poll_is_no_requests() {
        let mut server = mockito::Server::new();
        let _refresh = refresh(&mut server, "acc", "ref2");
        let _poll = server
            .mock("POST", "/v1/relay/poll")
            .with_body(r#"{"requests":[]}"#)
            .create();
        assert!(client(&server).poll("x").unwrap().is_empty());
    }

    #[test]
    fn explicit_capabilities_do_not_implicitly_enable_meetings() {
        let mut server = mockito::Server::new();
        let _refresh = refresh(&mut server, "acc", "ref2");
        let poll = server
            .mock("POST", "/v1/relay/poll")
            .match_body(Matcher::Json(
                json!({"client":"preview", "capabilities":["get_writing_profile"]}),
            ))
            .with_body(r#"{"requests":[]}"#)
            .create();
        assert!(
            client(&server)
                .poll_with_capabilities("preview", &["get_writing_profile".into()])
                .unwrap()
                .is_empty()
        );
        poll.assert();
    }

    #[test]
    fn source_selection_never_sends_a_caller_supplied_device_or_profile() {
        let mut server = mockito::Server::new();
        let _refresh = refresh(&mut server, "acc", "ref2");
        let read = server
            .mock("GET", "/v1/writing-style/source")
            .match_header("authorization", "Bearer acc")
            .with_body(r#"{"designated":false}"#)
            .create();
        let choose = server
            .mock("PUT", "/v1/writing-style/source")
            .match_body(Matcher::Json(json!({})))
            .with_body(r#"{"designated":true}"#)
            .create();
        let remove = server
            .mock("DELETE", "/v1/writing-style/source")
            .match_body(Matcher::Json(json!({})))
            .with_body(r#"{"designated":false}"#)
            .create();
        let relay = client(&server);
        assert!(!relay.writing_style_source(None).unwrap());
        assert!(relay.writing_style_source(Some(true)).unwrap());
        assert!(!relay.writing_style_source(Some(false)).unwrap());
        read.assert();
        choose.assert();
        remove.assert();
    }

    #[test]
    fn unauthorized_poll_refreshes_once_and_retries() {
        let mut server = mockito::Server::new();
        let first = refresh(&mut server, "old", "ref2");
        let rejected = server
            .mock("POST", "/v1/relay/poll")
            .match_header("authorization", "Bearer old")
            .with_status(401)
            .with_body(r#"{"error":"unauthorized","message":"Nee."}"#)
            .create();
        let relay = client(&server);
        relay.session.access_token().unwrap();
        first.remove();
        let _second = refresh(&mut server, "new", "ref3");
        let accepted = server
            .mock("POST", "/v1/relay/poll")
            .match_header("authorization", "Bearer new")
            .with_body(r#"{"requests":[]}"#)
            .create();
        assert!(relay.poll("x").unwrap().is_empty());
        rejected.assert();
        accepted.assert();
    }

    #[test]
    fn poll_error_maps_to_cloud_error() {
        let mut server = mockito::Server::new();
        let _refresh = refresh(&mut server, "acc", "ref2");
        let _poll = server
            .mock("POST", "/v1/relay/poll")
            .with_status(429)
            .with_body(r#"{"error":"rate_limited","message":"Te veel."}"#)
            .create();
        assert_eq!(
            client(&server).poll("x"),
            Err(PennelError::Cloud {
                code: "rate_limited".into(),
                message: "Te veel.".into()
            })
        );
    }

    #[test]
    fn respond_sends_result_or_error() {
        let mut server = mockito::Server::new();
        let _refresh = refresh(&mut server, "acc", "ref2");
        let text = server
            .mock("POST", "/v1/relay/respond")
            .match_body(Matcher::Json(
                json!({"id": "r1", "result": {"text": "# Hallo"}}),
            ))
            .with_status(204)
            .create();
        let failed = server
            .mock("POST", "/v1/relay/respond")
            .match_body(Matcher::Json(
                json!({"id": "r2", "error": {"code": "not_found", "message": "Weg."}}),
            ))
            .with_status(204)
            .create();
        let relay = client(&server);
        relay
            .respond(
                "r1",
                &RelayAnswer::Text {
                    text: "# Hallo".into(),
                },
            )
            .unwrap();
        relay
            .respond(
                "r2",
                &RelayAnswer::Failed {
                    code: "not_found".into(),
                    message: "Weg.".into(),
                },
            )
            .unwrap();
        text.assert();
        failed.assert();
    }

    #[test]
    fn respond_to_an_expired_request_is_not_an_error() {
        let mut server = mockito::Server::new();
        let _refresh = refresh(&mut server, "acc", "ref2");
        let _gone = server
            .mock("POST", "/v1/relay/respond")
            .with_status(404)
            .with_body(r#"{"error":"relay_unknown","message":"Onbekend."}"#)
            .create();
        let answer = RelayAnswer::Text { text: "x".into() };
        assert_eq!(client(&server).respond("r1", &answer), Ok(()));
    }
}
