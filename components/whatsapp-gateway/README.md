# asterbot:whatsapp-gateway

WhatsApp message handler for asterbot.
Receives incoming WhatsApp messages via the
`asterai:whatsapp` component and routes them through
the agent for a response.

## How it works

1. Receives a message via `asterai:whatsapp/incoming-handler`
2. Ignores messages from the bot itself (prevents loops)
3. Checks access control (see below)
4. Calls `agent::converse` with the message content
5. Sends the agent's response back to the sender

## Environment Variables

| Variable                  | Required | Description                                                     |
|---------------------------|----------|-----------------------------------------------------------------|
| `WHATSAPP_ALLOWED_PHONES` | No       | Comma-separated phone numbers allowed to interact with the bot. |
| `WHATSAPP_PUBLIC`         | No       | Set to `true` to allow all users. Defaults to `false`.          |

Example: `WHATSAPP_ALLOWED_PHONES=1234567890,0987654321`

Phone numbers should match the format WhatsApp uses
(digits only, no `+` prefix).

## Access control

The bot is **disabled by default**. You must configure
one of the following to enable it:

- Set `WHATSAPP_ALLOWED_PHONES` to restrict access
  to specific phone numbers.
- Set `WHATSAPP_PUBLIC=true` to allow all users.

If both are set, `WHATSAPP_ALLOWED_PHONES` takes
priority and a warning is logged.

Any WhatsApp user who messages the bot's number can
reach it — so public access should be enabled with
care.
