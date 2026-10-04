# BrainWashed

Turn your laptop into a private AI server, and use it from your phone or any browser, anywhere.

BrainWashed is one command, `brainwashed`. It runs open source language models on your own computer (macOS, Windows, Linux), lets you teach it new abilities by dropping in a markdown **skill** file, and serves a web app: chat for everyone, and admin pages to manage models, skills, devices, remote access and settings. A built-in secure tunnel makes it reachable from anywhere with no router setup. Optional iOS/Android apps connect to the same host. Your conversations stay end-to-end encrypted between your devices and hardware you own.

> **Status:** pre-release. See [install.md](docs/install.md) to try it, and the [architecture and roadmap](docs/architecture.md) for what's next.

## Install

- **Windows** (PowerShell): `irm https://raw.githubusercontent.com/ahmadalshouly/brainwashed/main/install.ps1 | iex`
- **macOS and Linux**: `curl -fsSL https://raw.githubusercontent.com/ahmadalshouly/brainwashed/main/install.sh | sh`

It starts right away and opens the admin page. Later, run `brainwashed`. [docs/install.md](docs/install.md) lists every command, including `brainwashed service install` to keep it running in the background.

## Repository layout

| Path | What it is |
|---|---|
| `crates/cli` | The `brainwashed` command: the host |
| `apps/web` | The web app the host serves: chat and admin pages |
| `packages/api` | TypeScript types and client for the host's client protocol |
| `crates/core` | The engine: models, llama.cpp runtime, skills and chat |
| `crates/skills` | Parser and loader for `SKILL.md` files |
| `crates/gateway` | The host's server: encrypted API for paired devices, roles, audit log, tunnel and relay connection |
| `crates/relay` | Relay server for using BrainWashed away from home ([docs](docs/relay.md)) |
| `skills-examples` | Example skills |
| `model` | Fine-tuning scripts and evals for the BrainWashed model |
| `docs` | Architecture and design notes |

## Getting started

Prerequisites: Node 20+, pnpm 10 and Rust (stable).

```sh
pnpm install
pnpm web:build       # build the web app; debug builds of the host read it from disk
pnpm dev             # run the host (cargo run -p brainwashed-cli); add -- --help for options
pnpm typecheck && pnpm test && cargo test --workspace
```

## Skills

A skill is a folder with a `SKILL.md` file: YAML frontmatter describing it, followed by instructions in plain markdown.

```markdown
---
name: email-writer
description: Drafts and rewrites emails in the right tone for the recipient.
triggers: [write an email, draft an email]
---
When the user wants an email written:
1. ...
```

How it works:

- Skills live in the data folder under `skills/`, one subfolder per skill (`brainwashed skills` shows where). Edit them on the admin page's **Skills** page or in any text editor; changes apply within seconds.
- Every enabled skill's `name` and `description` go into the system prompt as a short index.
- For each message the host picks at most two relevant skills and adds their full instructions. A `triggers` phrase in the message always selects a skill; otherwise skills are matched by the distinctive words they share with the message (and the previous message, so follow-ups keep their skill).
- Skills are never trained into the model, so a new skill works on the next message.

See [`skills-examples`](skills-examples) for the skills that ship with BrainWashed.

## Using it from your phone, another computer, or your team

No app is required: any browser works, at home or away.

1. On the admin page, open **Devices** and click **Show code**. Pick **member** (chat only) or **admin**.
2. Scan the QR code with the phone's camera, or open the link on another computer. It opens BrainWashed in the browser and connects it.
3. It stays connected until you remove it under **Devices**. The **Activity** page records who connected and every change admins make.

By default the code carries a free Cloudflare tunnel address, so it works from anywhere. For an address that never changes, use your own domain, Tailscale or a reverse proxy; see [docs/remote-access.md](docs/remote-access.md).

The optional BrainWashed iOS and Android apps, sold separately, scan the same code. Anyone can build their own client: the protocol is documented and versioned in [docs/client-protocol.md](docs/client-protocol.md), and `@brainwashed/api` implements it in TypeScript.

Messages are end-to-end encrypted with keys exchanged through the QR code, so the tunnel only ever carries ciphertext.

## Releasing

Maintainers: see [docs/releasing.md](docs/releasing.md).

## License

[Apache-2.0](LICENSE)
