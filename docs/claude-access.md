# Claude-toegang (asking Claude about your meetings)

Claude (claude.ai connectors, Claude Desktop, Claude Code, any MCP client) can search and read your meetings through Pennel Cloud's MCP server at `https://api.pennel.app/mcp`. Meetings stay end-to-end encrypted in Pennel Cloud: the server cannot read them. Claude's questions are therefore answered by one of your own computers, from its local meetings, and the answer passes through the server in memory only.

The authoritative design (decisions, security analysis) is in the pennel-cloud repository: `docs/superpowers/specs/2026-10-26-account-platform-mcp-design.md`. This file mirrors the part the app implements.

The [writing-profile connector](writing-profile.md) uses a separate `/mcp/style` connector and consent. Its profile permission does not authorize these meeting tools, and existing meeting permissions do not authorize writing-profile access.

## What the user does

1. Settings → Account → **Claude-toegang**: switch it on (off by default, per computer). Requires being signed in to Pennel Cloud.
2. In Claude, add the connector `https://api.pennel.app/mcp` (claude.ai: Settings → Connectors → Add custom connector), or `claude mcp add --transport http pennel https://api.pennel.app/mcp`. Claude opens pennel.app to sign in and asks for consent.

While the switch is on and the app runs, the computer keeps one long-poll request open to Pennel Cloud.

## Relay protocol (v1)

Authenticated with the device's normal access token (`Authorization: Bearer …`, refreshed like every other cloud call).

### `POST /v1/relay/poll`

Body `{"client": "pennel-desktop/<version>"}`. The server holds the request up to 25 s and answers

```json
{"requests": [{"id": "<uuid>", "tool": "search_meetings", "arguments": {"query": "budget", "limit": 10}, "timeout_ms": 20000}]}
```

with zero or one request. A new poll from the same device supersedes an older one. On `401` the app refreshes the token once and retries; on network errors it backs off (2 s doubling to 60 s).

### `POST /v1/relay/respond`

```json
{"id": "<uuid>", "result": {"text": "<markdown>"}}
{"id": "<uuid>", "error": {"code": "not_found", "message": "Deze meeting bestaat niet (meer) op deze computer."}}
```

`204` accepted, `404 relay_unknown` when the request already timed out. Text is cut at 200 000 characters with a note.

### Tools

| Tool | Arguments | Answer (Markdown text) | Engine functions |
|---|---|---|---|
| `search_meetings` | `query` (1–200 chars), `limit` 1–20 (default 10) | matching meetings with id, title, date, duration and up to two snippets | `search_library` |
| `list_recent_meetings` | `limit` 1–50 (default 10) | newest meetings with id, title, date, duration, whether there is a summary | `meetings` |
| `get_meeting` | `meeting_id` (must be one of this computer's meetings), `part` = `summary` / `notes` / `transcript` / `all` (default `all`) | that part; `summary` is the body of `meeting.md`, `notes` is `notes.md` | `meetings`, `meeting_markdown`, `meeting_notes`, `meeting_transcript` |
| `ask_meetings` | `question` (1–1000 chars), optional `meeting_id` | an answer citing the meetings it used; without a configured model, the most relevant passages | `ask_meetings`, then the meeting index for the fallback |

Error codes: `not_found`, `invalid_arguments`, `unavailable`, `failed`.

## Privacy rules the app follows

- Off by default on every computer; only polls while signed in **and** the switch is on. Signing out or deleting the account switches it off. After switching off, a poll already in flight ends on its own and any request it brings is answered `unavailable` instead of being run.
- At most four calls are answered at once; a fifth is answered `unavailable`.
- Implementation: `core/src/cloud/relay.rs` (HTTP), `core/src/claude_access.rs` (tools), `apps/desktop/src-tauri/src/claude_access.rs` (runner), `apps/desktop/src/views/account.ts` (UI).
- Read-only: the handlers call fixed engine functions with validated arguments. Nothing Claude sends can change, delete or export meetings or trigger any other action.
- Never logged: arguments, questions and answers. Logs hold the tool name, duration and outcome.
- Audio never leaves the computer through this channel; only text.
