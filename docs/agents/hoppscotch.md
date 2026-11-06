# Hoppscotch development tools

Use the official Hoppscotch MCP for **API workspace metadata**, and the owning
API repository's pinned CLI runner for HTTP checks. Pennel's meeting-access MCP
serves personal meeting content and is a separate product integration. Do not
connect it for this workflow or add HTTP wrappers around desktop Tauri commands.

## Connect

The project config pins `@hoppscotch/mcp-server@1.0.2` and requires Node.js 22
or newer. Codex reads `.codex/config.toml` in trusted projects; Claude Code reads
`.mcp.json` with the matching `.claude/settings.json` policy. Restart the host
after a configuration change and check `/mcp`. Do not register the server at user
or global scope: product CLI agents run outside this project and must not
inherit this development integration.

The first account-backed tool call opens Hoppscotch's device-login page. Finish
sign-in with your personal account in the browser. This is the server's own
login flow, **not** `codex mcp login`; a Hoppscotch PAT cannot authenticate it.
The server stores its session outside the repository at
`~/.config/hoppscotch-mcp/auth.json`, shared by MCP clients under the same OS
user. Never read, copy or commit that file. Use `reauth` only to intentionally
restart sign-in or switch identity, not while a login is already pending.

Use Personal Workspace with Pennel collections. Do not create a subscription,
paid organization or self-hosted service. Pricing lists MCP support under
Business, so a successful login alone does not establish free-account access.
If a read-only personal listing/export is denied by the account's entitlement,
disable this MCP in the host and retain the Desktop/Git/CLI workflow.

## Allowed work

The `core` server profile is narrowed by host policy to these twelve tools:

- `reauth`
- `list_user_collections`, `create_user_collection`, `update_user_collection`
- `import_user_collection`, `export_user_collection`
- `list_user_requests`, `create_user_request`, `update_user_request`
- `list_user_environments`, `create_user_environment`, `update_user_environment`

Codex has an explicit allowlist. Claude has the same explicit permissions and
denies the other 27 tools in the pinned release's core profile. These policies
block raw HTTP execution/validation, deletion, moves, code generation and team
access. Keep the server version and both host policies aligned when updating.
The profile remains `core`, strict environment loading is enabled, and private
host access remains disabled. Do not change other host permission settings.

Act only on the Pennel collection/environment IDs identified for this setup;
the signed-in identity is not restricted to Pennel by the server. Read-only
exports can include saved request credentials, so keep all saved Pennel requests
as synthetic templates with empty secret placeholders. Never export unrelated
collections or log their request data.

## Maintain the workspace

1. Edit the canonical request/environment files in the owning API repository
   and run its static checker and appropriate safe checks.
2. Export the known Pennel collections to inspect their IDs and compare the
   current definitions. On Cloud, use `export_user_collection` for a known ID;
   `get_user_collection` has an upstream serialization failure and is blocked.
3. Import only when initially creating a missing collection. Re-importing is
   **not an upsert** and creates duplicates. Update existing collections and
   requests by their IDs; create only missing objects.
4. Export again and review the normalized diff against Git. Desktop edits must
   return through a reviewed Git diff before becoming canonical.

Follow the owning repository's format conversion instructions before MCP imports:
the personal backend stores folder properties separately from Desktop's native
export. Use its export comparison tool to detect lost metadata; do not assume
that matching request counts establish a full round-trip.

Collection updates change properties, not the whole nested request tree.
Flag duplicate names, deletions, moves and ordering differences for explicit
reconciliation; this workflow is not an automatic exact mirror. Leave unrelated
workspace data alone.

Keep real credentials in Desktop's local secret variables or inject them
locally for an explicitly chosen CLI workflow. Desktop secret values do not
sync/export, and the MCP cannot read that local secret store. MCP environment
writes are for **nonsecret values and empty secret declarations only**, even
when a variable is marked `secret: true`. Updating an environment's variables
replaces the entire list; do not write masked secret values back.

Authenticated, paid and destructive API calls remain manual. The default agent
runner executes only the public smoke collection. Do not automatically refresh
product credentials or retry credential exchanges.

## Sources

- [Hoppscotch MCP setup and authentication](https://docs.hoppscotch.io/documentation/clients/mcp-server/overview)
- [Shipped tool definitions and profiles](https://github.com/hoppscotch/hoppscotch-mcp-server/blob/1.0.2/src/tools/definitions.ts)
- [Environment storage and export](https://docs.hoppscotch.io/documentation/features/environments)
- [Cloud pricing](https://hoppscotch.com/pricing)
- [Codex project-scoped MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)
- [Claude Code MCP permissions](https://code.claude.com/docs/en/permissions#mcp)
