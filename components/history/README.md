# asterbot:history

Default conversation history backend for asterbot. Persists the full conversation
as a single `conversation.json` file via `asterai:fs`, and provides automatic
compaction that summarises older messages into rolling context using an LLM call.

## Interface

Defined in `asterbot:types/history`:

| Function                | Description                                                           |
|-------------------------|-----------------------------------------------------------------------|
| `load()`                | Returns the working set (messages after the compaction cursor)        |
| `save(messages)`        | Merges the working set with the archived portion and writes           |
| `clear()`               | Deletes `conversation.json` entirely                                  |
| `get-context()`         | Returns assembled summary context for the system prompt               |
| `should-compact(count)` | Checks if the working set exceeds the compaction threshold            |
| `compact(messages)`     | Summarises old messages via LLM, advances cursor, returns trimmed set |

## File format

Everything lives in a single `conversation.json`:

```json
{
  "history": [],
  "compactedThrough": 0,
  "conversationSummary": "",
  "userSummary": "",
  "bondSummary": ""
}
```

- `history` — Full message archive, append-only, never truncated.
- `compactedThrough` — Cursor index. Messages before this have been summarised.
- `conversationSummary` — Rolling narrative of the conversation so far.
- `userSummary` — Observed user profile (name, preferences, technical level, etc.).
- `bondSummary` — Notes on the user-assistant relationship dynamics.

Old-format or malformed files are reset to empty state (no backward compatibility).

## Compaction

When the working set exceeds the threshold, `compact()`:

1. Splits the working set into old messages (to summarise) and recent messages (to keep).
2. Calls the LLM with a structured tool (`update_context`) to produce updated summaries.
3. Advances `compactedThrough` and writes the updated state.
4. Returns the trimmed working set to core.

Core calls `compact()` **before** the main LLM call, so the current turn benefits
from the shorter context window. Compaction currently blocks the response — there is
a TODO to run it asynchronously via `asterai:host-cron`.

The compaction LLM call uses structured tool calling (not XML parsing) for reliable
output extraction.

## Configuration

| Env var                         | Default      | Description                                                         |
|---------------------------------|--------------|---------------------------------------------------------------------|
| `ASTERBOT_COMPACTION_THRESHOLD` | `50`         | Message count that triggers compaction                              |
| `ASTERBOT_KEEP_RECENT_TURNS`    | `10`         | User turns to keep after compaction                                 |
| `ASTERBOT_MODEL`                | *(required)* | Model for the compaction LLM call. If empty, compaction is skipped. |

### Relationship with core's prompt trimming

Core has a separate `trim_history` safety net controlled by:

| Env var                             | Default | Description                                    |
|-------------------------------------|---------|------------------------------------------------|
| `ASTERBOT_MAX_PROMPT_USER_MESSAGES` | -       | Hard cap on user messages in the LLM prompt    |
| `ASTERBOT_MAX_PROMPT_CHARS`         | -       | Hard cap on total characters in the LLM prompt |

These are independent of compaction. Recommended: set `MAX_PROMPT_USER_MESSAGES`
to ~2x `KEEP_RECENT_TURNS` (e.g. 20) so trim only kicks in as an emergency backstop.
Even if trim discards messages that compaction kept, the conversation summary from
`get-context()` still covers them.

## Dependencies

- `asterai:fs` — File persistence (swappable: local fs, S3, Google Drive, etc.)
- `asterai:llm` — LLM calls for compaction summarisation
