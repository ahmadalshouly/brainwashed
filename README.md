# BrainWashed

Turn your laptop into a private AI server, and talk to it from your phone or any browser.

BrainWashed runs open source language models on your own computer (macOS, Windows, Linux), lets you teach it new abilities by dropping in a markdown **skill** file, and serves a web chat so you can use it from your phone or any other device in a browser. Optional iOS/Android apps connect to the same host. Your conversations never leave hardware you own.

> **Status:** pre-release. The desktop host, skills, the web chat, pairing other devices and access away from home through a relay all work; installers are built for every release but not yet signed. See [install.md](docs/install.md) to try it, and the [architecture and roadmap](docs/architecture.md) for what's next.

## Install

Download BrainWashed for macOS, Windows or Linux from the [releases page](https://github.com/ahmadalshouly/brainwashed/releases). [docs/install.md](docs/install.md) explains which file to pick and how to get past the warning for unsigned installers.

## Repository layout

| Path | What it is |
|---|---|
| `apps/host` | Desktop host app (Tauri 2: Rust core + React UI) |
| `apps/web` | Web chat the host serves to browsers on your network |
| `packages/api` | TypeScript types and client for the host's client protocol |
| `crates/cli` | `brainwashed` command: the host without a window, for terminals and servers |
| `crates/skills` | Parser and loader for `SKILL.md` files |
| `crates/gateway` | The host's encrypted API for paired devices, and its relay connection |
| `crates/relay` | Relay server for using BrainWashed away from home ([docs](docs/relay.md)) |
| `skills-examples` | Example skills |
| `model` | Fine-tuning scripts and evals for the BrainWashed model |
| `docs` | Architecture and design notes |

## Getting started

Prerequisites: Node 20+, pnpm 10, Rust (stable), and the [Tauri system dependencies](https://v2.tauri.app/start/prerequisites/) for your OS.

```sh
pnpm install
pnpm host:dev        # run the desktop host
pnpm web:build       # build the web chat the host serves (rebuild the host after)
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

- Skills live in the app's data folder under `skills/`, one subfolder per skill. Edit them in the app's Skills tab or in any text editor; changes apply within seconds.
- Every enabled skill's `name` and `description` go into the system prompt as a short index.
- For each message the host picks at most two relevant skills and adds their full instructions. A `triggers` phrase in the message always selects a skill; otherwise skills are matched by the distinctive words they share with the message (and the previous message, so follow-ups keep their skill).
- Skills are never trained into the model, so a new skill works on the next message.

See [`skills-examples`](skills-examples) for the skills that ship with the app.

## Using it from your phone or another computer

No app is required: the host serves a web chat that works in any browser on your network.

1. On the computer, open **Devices**, turn on **Allow phones and browsers on this network**, then click **Pair a device**.
2. Scan the QR code with your phone's camera. It opens the chat in the browser and pairs it. On another computer, open the link shown under the code.
3. Chat and switch models from that browser. It stays paired until you remove it under **Devices**.

The optional BrainWashed iOS and Android apps, sold separately, scan the same QR code and add more on top. Anyone can build their own client: the protocol is documented and versioned in [docs/client-protocol.md](docs/client-protocol.md), and `@brainwashed/api` implements it in TypeScript.

Messages are end-to-end encrypted with keys exchanged through the QR code.

### Away from home

Under **Devices > Away from home**, point the computer at a relay. Paired apps then keep working on any network, with no router setup, and the relay only ever sees encrypted traffic. You can run your own relay with one Docker command; see [docs/relay.md](docs/relay.md). The web chat stays on your home network.

## Releasing

Maintainers: see [docs/releasing.md](docs/releasing.md).

## License

[Apache-2.0](LICENSE)
