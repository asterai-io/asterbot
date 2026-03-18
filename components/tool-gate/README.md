# asterbot:tool-gate

Security gate component for asterbot. Controls which tool calls the AI agent
can execute by enforcing a permission model where every tool function is either
pre-authorized or requires user approval before execution.

Implements the `tool-hook` interface (called by toolkit before/after every tool
call) and `api` (for managing permissions and approving pending requests
via the runtime's call endpoint).

## How it works

Every tool function falls into one of two configured states, or the default:

- **allow** — the function executes immediately.
- **deny** — the function is permanently blocked. Must be changed from the UI.
- **not listed** (default) — the gate blocks and waits for the user to approve
  or deny via the api.

All state is stored in the `tool-gate/` directory and encrypted with AES-256-GCM
using the configured secret. If any file is missing, corrupted, or cannot be
decrypted, the gate falls back to safe defaults (all functions require approval).

## Security model

- **Encryption** — all files in `tool-gate/` are encrypted. The LLM cannot
  read or write meaningful content even if it has filesystem access.
- **Nonce-based replay protection** — each pending request contains a unique
  nonce (UUIDv7) held in process memory during the blocking poll. When the
  user confirms, the nonce is preserved in the updated encrypted file. The
  polling loop verifies the nonce matches before accepting. A replayed file
  from a previous session will have a stale nonce that doesn't match.

## Workflow example

The LLM wants to call `email/send` and it is not pre-authorized:

1. Core's agent loop: LLM returns a tool call for `email/send` with args
   `{to: "alice@example.com", subject: "Hello", body: "..."}`.
2. Core calls `toolkit/call-tool`.
3. Toolkit discovers the tool-gate component (it exports `tool-hook`).
4. Toolkit calls `gate.before-call("email-component", "email/send", args)`.
5. Gate checks permissions — `email/send` is not listed (defaults to require
   approval).
6. Gate generates a nonce (UUIDv7), holds it in a local variable.
7. Gate writes an encrypted pending request to `tool-gate/pending/{id}.bin`.
8. Gate starts polling the file for a status change.
9. The agent is now blocked here.
10. UI polls `api/list-pending` via the runtime call endpoint, sees the
    request (decrypted and returned without the nonce).
11. UI presents it to the user: "Aster wants to send an email to
    alice@example.com — [Approve Once] [Always Allow] [Deny]".
12. User clicks one of the options.
13. UI calls `api/confirm` with the request ID and the user's choice.
14. Gate reads the encrypted pending file, updates the status, re-encrypts
    and writes it back (and if "Always Allow", updates permissions too).
15. Gate's polling loop reads the file, verifies the nonce matches, sees the
    new status.
16. Gate returns `allow` or `deny` to toolkit.
17. Toolkit proceeds with or rejects the tool call.

From core's perspective, nothing special happened — `call_tool` just took a
while to return.

## Startup cleanup

On first invocation after deployment, the gate clears all pending requests.
If the agent was redeployed mid-block, the LLM loop that initiated the tool
call is gone, so pending approvals are meaningless.

## Gate API

Callable through the asterai runtime's call endpoint:

- `list-pending() -> string` — list pending approval requests as JSON.
- `confirm(id, action) -> string` — resolve a pending request.
  Action: `"approve_once"`, `"approve_always"`, or `"deny"`.
- `get-permissions() -> string` — list current function permissions as JSON.
- `update-permission(key, value) -> string` — set a permission.
  Key: `"component/function"`, value: `"allow"` or `"deny"`.
- `remove-permission(key) -> string` — remove a permission (reverts to
  requiring approval).

## Configuration

- `ASTERBOT_TOOL_GATE_SECRET` — encryption key for all gate state. Required
  for production use.
- `ASTERBOT_TOOL_GATE_TIMEOUT` — seconds to wait for user approval before
  auto-denying (default: 300).

## Interfaces

- Exports: `asterbot:types/tool-hook`, `api`.
- Imports: `asterai:host/api`, `asterai:fs/fs`.
