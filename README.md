<br />
<p align="center">
  <a href="https://asterai.io">
    <img src="assets/asterbot.png" alt="Asterbot" width="100">
  </a>
</p>

<h3 align="center"><b>Asterbot</b></h3>
<p align="center">
    <b>A hyper-modular AI agent built on WASM components.</b><br />
    Every capability is a swappable component. Written in any language.
</p>

<div align="center">

[![License](https://img.shields.io/github/license/asterai-io/asterbot?color=blue)](https://github.com/asterai-io/asterbot/blob/master/LICENSE)
[![Discord](https://img.shields.io/discord/1260408236578832475?label=discord&color=7289da)](https://asterai.io/discord)
[![GitHub stars](https://img.shields.io/github/stars/asterai-io/asterbot)](https://github.com/asterai-io/asterbot)
[![X Follow](https://img.shields.io/twitter/follow/asterai_io)](https://x.com/asterai_io)

</div>

<h4 align="center">
  <a href="https://asterai.io" target="_blank">Website</a> ·
  <a href="https://docs.asterai.io" target="_blank">Documentation</a> ·
  <a href="https://asterai.io/discord" target="_blank">Discord</a>
</h4>

<br />

## ✨ Overview

Asterbot is a hyper-modular AI agent built on WASM components.

Think microkernel architecture for AI agents. Asterbot is just the orchestration
core. LLM calls, tools, memory, and planning are all swappable WASM components
that can be retrieved and discovered from the public
[asterai](https://github.com/asterai-io/asterai) WASM component registry.

## 🏗 Architecture

```
User
 │
 │  converse("hello")
 ▼
┌──────────────────┐
│  asterbot:agent   │  Stable entrypoint. Delegates to core.
└────────┬─────────┘
         │  call-component-function (dynamic dispatch)
         ▼
┌──────────────────┐
│  asterbot:core    │  The brain. Agent loop: build prompt,
└──┬──────┬────────┘  call LLM, parse tool calls, loop.
   │      │
   │      ▼
   │  ┌──────────────────┐
   │  │ asterbot:toolkit  │  Discovers tools in the environment
   │  └──┬───────────────┘  via host API reflection.
   │     │
   │     ▼
   │  ┌──────────────────┐
   │  │ Tool components   │  Any WASM component: web search,
   │  │ (user-provided)   │  memory, skills, soul, APIs, ...
   │  └──────────────────┘
   │
   ▼
┌──────────────────┐
│  asterai:llm      │  12 LLM providers. One interface.
└──────────────────┘
```

All inter-component calls use dynamic dispatch (`call-component-function`
with JSON args). No component knows about the others at compile time —
swap any piece by changing an env var.

## 🚀 Quick Start

### Install the CLI
There are two options, both will install the same CLI:

NPM users:
```
npm install -g @asterai/cli
```

Cargo/Rust users:
```bash
cargo install asterai
```

### Setup asterbot

Create an environment and add the core components:

```bash
asterai env init asterbot

# Core
asterai env add-component asterbot asterbot:agent
asterai env add-component asterbot asterbot:core
asterai env add-component asterbot asterbot:toolkit
asterai env add-component asterbot asterai:llm

# Core tools
asterai env add-component asterbot asterbot:soul
asterai env add-component asterbot asterbot:memory
asterai env add-component asterbot asterbot:skills

# Other tools (example -- you can add any component as a tool)
asterai env add-component asterbot asterai:firecrawl
```

Configure:

```bash
# LLM provider (pick any: OpenAI, Anthropic, Mistral, etc.)
asterai env set-var asterbot --var ASTERBOT_MODEL="anthropic/claude-sonnet-4-5"
asterai env set-var asterbot --var ANTHROPIC_KEY="sk-..."

# Enable tools the agent can use
# This will also enable the Firecrawl component as a tool.
asterai env set-var asterbot --var ASTERBOT_TOOLS="asterbot:soul,asterbot:memory,asterbot:skills,asterai:firecrawl"

# Firecrawl API key (for web search/scrape)
asterai env set-var asterbot --var FIRECRAWL_KEY="fc-..."
```

Run:

```bash
asterai env call asterbot --allow-dir ~/.asterbot \
  asterbot:agent agent/converse "hello!"
```

The `--allow-dir` flag grants the agent filesystem access for
persistent memory, skills, and conversation history.

### Example

```
$ asterai env call asterbot --allow-dir ~/.asterbot \
    asterbot:agent agent/converse \
    "hi! can you remember my favourite programming language is rust"

calling env lorenzo:asterbot's asterbot:agent component function agent/converse
allowed directories:
  /home/lorenzo/.asterbot
compiling asterbot:core@1.0.0... done.
compiling asterbot:memory@1.0.0... done.
compiling asterai:firecrawl@0.1.0... done.
compiling asterbot:soul@1.0.0... done.
compiling asterbot:skills@1.0.0... done.
compiling asterai:llm@1.0.0... done.
compiling asterbot:toolkit@1.0.0... done.
compiling asterbot:agent@1.0.0... done.

Hi! 👋 I've saved that your favorite programming language is Rust!
That's a great choice - Rust is known for its memory safety, performance,
and excellent tooling. I'll remember this for our future conversations.
Is there anything else you'd like me to help you with?
```

The agent used the memory tool to persist this. We can inspect the
state directory:

```
$ ls ~/.asterbot/
conversation.json  memory/

$ cat ~/.asterbot/memory/user_favorite_programming_language.md
Rust
```

## 🧩 How it works

Asterbot runs on [asterai](https://github.com/asterai-io/asterai), an open-source
WASM component runtime and registry. Components are compiled to WASM, published to
the registry, and composed into environments at runtime.

Any component in the registry can be added as a tool. Write a component in Rust, Go,
Python, or any language that compiles to WASM, publish it, and asterbot can call it.
Components communicate through typed WIT interfaces and are sandboxed via WASI --
they can't access host resources unless explicitly granted.

All asterbot components are published to the registry and can be browsed at
[asterai.io/asterbot](https://asterai.io/asterbot)
(e.g. [asterbot:memory](https://asterai.io/asterbot/memory)).

## 🔑 Why Asterbot

- **Modular by default**: Swap out any piece
  (LLM provider, tools, memory) without touching the rest.
- **Secure**: Every component runs in a WASI sandbox. No full host access.
- **Polyglot**: Components can be written in
  Rust, Go, Python, JS, C/C++ -- all interoperate
  via typed WIT interfaces.
- **Portable**: Same components run locally or in the cloud,
  no environment-specific config.
- **Fast**: Rust core + near-native WASM execution.
  Sub-millisecond component instantiation.

## 🌟⚔🦞 Asterbot vs OpenClaw

|                  | Asterbot                          | OpenClaw                                     |
|------------------|-----------------------------------|----------------------------------------------|
| Language         | Any (via WASM)                    | TypeScript only                              |
| Tool security    | WASI sandboxed per component      | Full host access ([341 malicious skills][1]) |
| Tool portability | Framework-agnostic, runs anywhere | OpenClaw-only                                |
| Registry         | asterai -- any language           | ClawHub -- TypeScript only                   |
| Architecture     | Thin core + swappable components  | Monolithic TypeScript monorepo               |

[1]: https://thehackernews.com/2026/02/researchers-find-341-malicious-clawhub.html

## ⭐ Star History

[![Star History Chart](https://api.star-history.com/svg?repos=asterai-io/asterbot&type=Date)](https://star-history.com/#asterai-io/asterbot&Date)

## 📄 License

[APACHE 2.0](LICENSE)
