# BrainWashed

Turn your laptop into a private AI server, and talk to it from your phone.

BrainWashed runs open source language models on your own computer (macOS, Windows, Linux), lets you teach it new abilities by dropping in a markdown **skill** file, and pairs with an iOS/Android app so you can use your home AI from anywhere. Your conversations never leave hardware you own.

> **Status:** early development (Phase 0: foundations). Nothing is usable yet. See the [architecture and roadmap](docs/architecture.md).

## Repository layout

| Path | What it is |
|---|---|
| `apps/host` | Desktop host app (Tauri 2: Rust core + React UI) |
| `apps/mobile` | Phone app (React Native + Expo) |
| `packages/api` | TypeScript types and client shared by the host UI and phone app |
| `crates/skills` | Parser and loader for `SKILL.md` files |
| `skills-examples` | Example skills |
| `model` | Fine-tuning scripts and evals for the BrainWashed model |
| `docs` | Architecture and design notes |

## Getting started

Prerequisites: Node 20+, pnpm 10, Rust (stable), and the [Tauri system dependencies](https://v2.tauri.app/start/prerequisites/) for your OS.

```sh
pnpm install
pnpm host:dev        # run the desktop host
pnpm mobile:start    # run the phone app in Expo Go or a simulator
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

## Using it from your phone

1. On the computer, open **Phones**, turn on **Allow phones on this network**, then click **Pair a phone**.
2. In the BrainWashed phone app, tap **Pair a computer** and scan the QR code.
3. Chat, switch models and turn skills on or off from the phone.

Phone and computer must be on the same network for now. Traffic is end-to-end encrypted with keys exchanged through the QR code; see [docs/phone-protocol.md](docs/phone-protocol.md).

To try the phone app without a phone, run `pnpm --filter @brainwashed/mobile web` and paste the pairing link instead of scanning it.

## License

[Apache-2.0](LICENSE)
