# asterbot:telegram-gateway

Telegram message handler for asterbot.
Receives incoming Telegram messages via the
`asterai:telegram` component and routes them through
the agent for a response.

## How it works

1. Receives a message via `asterai:telegram/incoming-handler`
2. Ignores messages from the bot itself (prevents loops)
3. Checks access control (see below)
4. Calls `agent::converse` with the message content
5. Sends the agent's response back to the same chat

## Environment Variables

| Variable                    | Required | Description                                                         |
|-----------------------------|----------|---------------------------------------------------------------------|
| `TELEGRAM_ALLOWED_USER_IDS` | No       | Comma-separated Telegram user IDs allowed to interact with the bot. |
| `TELEGRAM_PUBLIC`           | No       | Set to `true` to allow all users. Defaults to `false`.              |

To find your Telegram user ID, message
[@userinfobot](https://t.me/userinfobot) on Telegram
— it will reply with your numeric ID.

Example: `TELEGRAM_ALLOWED_USER_IDS=123456789,987654321`

## Access control

The bot is **disabled by default**. You must configure
one of the following to enable it:

- Set `TELEGRAM_ALLOWED_USER_IDS` to restrict access
  to specific users.
- Set `TELEGRAM_PUBLIC=true` to allow all users.

If both are set, `TELEGRAM_ALLOWED_USER_IDS` takes
priority and a warning is logged.

Unlike Discord (where bots can only be messaged by
users in shared servers), any Telegram user who knows
the bot's username can message it directly — so
public access should be enabled with care.
