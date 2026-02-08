<br />
<p align="center">
  <a href="https://asterai.io">
    <img src="assets/asterbot.png" alt="Asterbot" width="100">
  </a>
</p>

<h3 align="center"><b>Asterbot</b></h3>
<p align="center">
    <b>A hyper-modular AI agent built on WASM components.</b><br />
    Every capability is a swappable component from the asterai registry.
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
core. LLM calls, tool,s memory, and planning are all swappable WASM components
that can be retrieved and discovered from the public
[asterai](https://github.com/asterai-io/asterai) WASM component registry.  

## 🔑 Why Asterbot

- **Modular by default**: Swap out any piece
  (LLM provider, tools, memory) without touching the rest
- **Secure**: Every component runs in a WASI sandbox.
  No full host access, no malicious tools
- **Polyglot**: Components can be written in
  Rust, Go, Python, JS, C/C++ -- all interoperate
  via typed WIT interfaces
- **Portable**: Same components run locally or in the cloud,
  no environment-specific config
- **Fast**: Rust core + near-native WASM execution.
  Sub-millisecond component instantiation

## 🌟⚔🦞 Asterbot vs OpenClaw 

|                  | Asterbot                          | OpenClaw                                     |
|------------------|-----------------------------------|----------------------------------------------|
| Language         | Any (via WASM)                    | TypeScript only                              |
| Tool security    | WASI sandboxed per component      | Full host access ([341 malicious skills][1]) |
| Tool portability | Framework-agnostic, runs anywhere | OpenClaw-only                                |
| Registry         | asterai -- any language           | ClawHub -- TypeScript only                   |
| Architecture     | Thin core + swappable components  | Monolithic TypeScript monorepo               |

[1]: https://thehackernews.com/2026/02/researchers-find-341-malicious-clawhub.html

## 🚀 Getting Started

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

TODO

## 📄 License

[APACHE 2.0](LICENSE)
