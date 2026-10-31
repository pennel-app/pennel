use std::time::Duration;

use serde_json::Value;

use crate::error::PennelError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers(Vec<(String, String)>);

impl Headers {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub fn agent(timeout_secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .http_status_as_error(false)
        .build()
        .into()
}

pub fn error_from(status: u16, body: &Value) -> PennelError {
    let code = body.get("error").and_then(Value::as_str);
    let message = body.get("message").and_then(Value::as_str);
    PennelError::Cloud {
        code: code.unwrap_or("unknown").to_string(),
        message: message.map_or_else(
            || format!("Pennel Cloud returned an error ({status})."),
            str::to_string,
        ),
    }
}

fn network(err: impl std::fmt::Display) -> PennelError {
    PennelError::Network {
        message: err.to_string(),
    }
}

fn dispatch(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    token: Option<&str>,
    headers: &[(&str, String)],
    content_type: &str,
    body: Option<&[u8]>,
) -> Result<ureq::http::Response<ureq::Body>, PennelError> {
    let method = ureq::http::Method::from_bytes(method.as_bytes()).map_err(network)?;
    let mut request = ureq::http::Request::builder()
        .method(method)
        .uri(url)
        .header("Accept", "application/json")
        .header(
            "User-Agent",
            concat!("pennel-core/", env!("CARGO_PKG_VERSION")),
        );
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    for (name, value) in headers {
        request = request.header(*name, value.as_str());
    }
    if body.is_some() {
        request = request.header("Content-Type", content_type);
    }
    let request = request
        .body(body.map(<[u8]>::to_vec).unwrap_or_default())
        .map_err(network)?;
    agent.run(request).map_err(network)
}

fn collect_headers(response: &ureq::http::Response<ureq::Body>) -> Headers {
    Headers(
        response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
            })
            .collect(),
    )
}

/// Returns the status and parsed JSON body of any response; an empty body becomes `Null`.
pub fn send_json(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Result<(u16, Value), PennelError> {
    let payload = body.map(Value::to_string);
    let mut response = dispatch(
        agent,
        method,
        url,
        token,
        &[],
        "application/json",
        payload.as_deref().map(str::as_bytes),
    )?;
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().map_err(network)?;
    if text.trim().is_empty() {
        return Ok((status, Value::Null));
    }
    match serde_json::from_str(&text) {
        Ok(value) => Ok((status, value)),
        // Proxies and gateways answer errors with HTML; callers turn Null into the generic message.
        Err(_) if !(200..300).contains(&status) => Ok((status, Value::Null)),
        Err(_) => Err(PennelError::InvalidResponse {
            message: format!("expected JSON (status {status}, {} bytes)", text.len()),
        }),
    }
}

/// Returns the status, headers and raw body of any response; callers decide how to treat non-2xx.
pub fn send_bytes(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<&[u8]>,
) -> Result<(u16, Headers, Vec<u8>), PennelError> {
    send_raw(
        agent,
        method,
        url,
        token,
        &[],
        "application/octet-stream",
        body,
    )
}

/// Like [`send_bytes`], with extra request headers and an explicit content type.
pub fn send_raw(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    token: Option<&str>,
    headers: &[(&str, String)],
    content_type: &str,
    body: Option<&[u8]>,
) -> Result<(u16, Headers, Vec<u8>), PennelError> {
    let mut response = dispatch(agent, method, url, token, headers, content_type, body)?;
    let status = response.status().as_u16();
    let headers = collect_headers(&response);
    let bytes = response
        .body_mut()
        .with_config()
        .limit(256 * 1024 * 1024)
        .read_to_vec()
        .map_err(network)?;
    Ok((status, headers, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn send_json_sends_bearer_and_parses_error_bodies() {
        let mut server = mockito::Server::new();
        let ok = server
            .mock("POST", "/v1/x")
            .match_header("authorization", "Bearer tok")
            .match_body(mockito::Matcher::Json(json!({"a": 1})))
            .with_status(200)
            .with_body(r#"{"ok":true}"#)
            .create();
        let failed = server
            .mock("GET", "/v1/y")
            .with_status(401)
            .with_body(r#"{"error":"unauthorized","message":"Nee."}"#)
            .create();
        let agent = agent(5);
        let (status, body) = send_json(
            &agent,
            "POST",
            &format!("{}/v1/x", server.url()),
            Some("tok"),
            Some(&json!({"a": 1})),
        )
        .unwrap();
        assert_eq!((status, body), (200, json!({"ok": true})));
        let (status, body) =
            send_json(&agent, "GET", &format!("{}/v1/y", server.url()), None, None).unwrap();
        assert_eq!(status, 401);
        assert_eq!(
            error_from(status, &body),
            PennelError::Cloud {
                code: "unauthorized".into(),
                message: "Nee.".into()
            }
        );
        ok.assert();
        failed.assert();
    }

    #[test]
    fn non_json_error_body_maps_to_generic_cloud_error() {
        let mut server = mockito::Server::new();
        let _m = server
            .mock("GET", "/v1/z")
            .with_status(502)
            .with_body("<html>Bad Gateway</html>")
            .create();
        let _ok = server
            .mock("GET", "/v1/w")
            .with_status(200)
            .with_body("<html>")
            .create();
        let agent = agent(5);
        let (status, body) =
            send_json(&agent, "GET", &format!("{}/v1/z", server.url()), None, None).unwrap();
        assert_eq!(
            error_from(status, &body),
            PennelError::Cloud {
                code: "unknown".into(),
                message: "Pennel Cloud returned an error (502).".into()
            }
        );
        assert!(matches!(
            send_json(&agent, "GET", &format!("{}/v1/w", server.url()), None, None),
            Err(PennelError::InvalidResponse { .. })
        ));
    }

    #[test]
    fn send_bytes_returns_headers_and_body() {
        let mut server = mockito::Server::new();
        let _m = server
            .mock("PUT", "/blob")
            .match_body(mockito::Matcher::Exact("abc".into()))
            .with_status(200)
            .with_header("ETag", "\"v1\"")
            .with_body(vec![1u8, 2, 3])
            .create();
        let (status, headers, bytes) = send_bytes(
            &agent(5),
            "PUT",
            &format!("{}/blob", server.url()),
            None,
            Some(b"abc"),
        )
        .unwrap();
        assert_eq!(status, 200);
        assert_eq!(headers.get("etag"), Some("\"v1\""));
        assert_eq!(bytes, vec![1, 2, 3]);
    }

    #[test]
    fn error_from_uses_server_message() {
        for message in [
            "Deze sessie is ingetrokken. Log opnieuw in.",
            "This session was revoked. Sign in again.",
        ] {
            let body = json!({"error": "session_revoked", "message": message});
            assert_eq!(
                error_from(401, &body),
                PennelError::Cloud {
                    code: "session_revoked".into(),
                    message: message.into()
                }
            );
        }
        assert_eq!(
            error_from(502, &json!({"error": "bad_gateway"})),
            PennelError::Cloud {
                code: "bad_gateway".into(),
                message: "Pennel Cloud returned an error (502).".into()
            }
        );
    }
}
