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

A skill is a folder with a `SKILL.md` file: YAML frontmatter describing it, followed by instructions in plain markdown. The host picks the relevant skill for each message and adds it to the model's prompt, so a new skill works immediately with no retraining. See [`skills-examples/meal-planner`](skills-examples/meal-planner/SKILL.md).

## License

[Apache-2.0](LICENSE)
