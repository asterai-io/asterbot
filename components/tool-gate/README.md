# asterbot:tool-gate

Security gate component for asterbot. Controls which tool calls the AI agent
can execute by enforcing a permission model where every tool function is either
pre-authorized or requires user approval before execution.

Implements the `tool-hook` interface (called by toolkit before/after every tool
call) and `wasi:http/incoming-handler` (serves a UI API for managing permissions
and approving pending requests).

## How it works

The gate classifies every tool function into one of three states:

- **Pre-authorized** — the function executes immediately (`allow`).
- **Requires approval** — the gate blocks and waits for the user to approve or
  deny via the HTTP API.
- **Denied** — the function is permanently blocked (`deny`). The user must
  change this from the UI.

Permissions are stored in an HMAC-signed file via `asterai:fs`. If the file is
missing, corrupted, or the signature is invalid, all functions default to
requiring approval (fail-safe).

## Workflow example

The LLM wants to call `email/send` and it is not pre-authorized:

1. Core's agent loop: LLM returns a tool call for `email/send` with args
   `{to: "alice@example.com", subject: "Hello", body: "..."}`.
2. Core calls `toolkit/call-tool`.
3. Toolkit iterates hooks, calls `gate.before-call("email-component",
   "email/send", args)`.
4. Gate checks the permissions file — `email/send` is not pre-authorized.
5. Gate writes a pending request to the filesystem:
   `{id: "abc123", component, function, args, status: "pending"}`.
6. Gate starts polling the filesystem for a response to `abc123`.
7. The agent is now blocked here.
8. UI polls `GET /pending` on the gate's HTTP endpoint, sees the request.
9. UI presents it to the user: "Aster wants to send an email to
   alice@example.com — [Approve Once] [Always Allow] [Deny]".
10. User clicks one of the options.
11. UI sends `POST /confirm/abc123` with the user's choice.
12. Gate's HTTP handler writes the response to the filesystem (and if
    "Always Allow", updates the permissions file).
13. Gate's polling loop picks up the response.
14. Gate returns `allow` or `deny` to toolkit.
15. Toolkit proceeds with or rejects the tool call — business as usual.

From core's perspective, nothing special happened — `call_tool` just took a
while to return.

## Startup

On startup (`wasi:cli/run`), the gate clears all pending requests from the
filesystem. If the agent was redeployed mid-block, the LLM loop that initiated
the tool call is gone, so pending approvals are meaningless. If the action was
important, the user will ask the agent again.

## HTTP API

The gate component implements `wasi:http/incoming-handler` to serve:

- `GET /pending` — list all pending approval requests.
- `POST /confirm/:id` — approve or deny a pending request.
  Body: `{"action": "approve_once" | "approve_always" | "deny"}`.
- `GET /permissions` — list current function permissions.
- `POST /permissions` — update function permissions.

## Configuration

- `ASTERBOT_TOOL_GATE_SECRET` — HMAC secret for signing the permissions file.
  If unset, permissions are stored unsigned (development only).

## Interfaces

- Exports: `asterbot:types/tool-hook`, `wasi:http/incoming-handler`,
  `wasi:cli/run`.
- Imports: `asterai:host/api`, `asterai:fs/fs`.
