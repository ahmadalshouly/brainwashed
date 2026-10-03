# BrainWashed: Architecture and Build Plan

_Draft 1, 2026-10-03. Turns a personal laptop into a private AI host, with a phone app that talks to it from anywhere._

## 1. What we are building

| Piece | What it does | Recommended stack |
|---|---|---|
| **Host app** (macOS, Windows, Linux) | Tray app that downloads and runs models, loads skills, serves an API, pairs phones | Tauri 2 (Rust core + React/TypeScript UI) |
| **Model runtime** | Runs any open GGUF model on CPU or GPU | llama.cpp `llama-server`, bundled as a sidecar |
| **Skills engine** | Loads user-written `.md` skills into the model at inference time | Rust module in the host, small embedding model for routing |
| **Connectivity** | Phone reaches the laptop at home or away, end-to-end encrypted | LAN first (QR pairing), then a self-hostable relay that forwards end-to-end encrypted traffic |
| **Web chat** | Chat from any browser on the network, paired by QR | React app served by the host (`apps/web`) |
| **Mobile app** (iOS, Android) | Optional paid client: chat, pick model, manage skills | React Native + Expo, closed source in a separate repo; uses only the public [client protocol](client-protocol.md) |
| **BrainWashed model** | Ahmad's fine-tuned 2-3B model, the default download | QLoRA fine-tune of an Apache-2.0 base, shipped as GGUF Q4_K_M (~2 GB) |

Why these choices:
- **Tauri over Electron:** ~10 MB installers instead of ~150 MB, Rust core is a good home for process management and networking, one codebase for all three desktops.
- **llama.cpp over writing our own runtime:** it already covers Metal (Apple), CUDA, ROCm and Vulkan (most Windows/Linux GPUs) and CPU, reads GGUF (the format nearly every open model on Hugging Face ships in), and exposes an OpenAI-compatible HTTP API. Ollama is an option too, but bundling llama.cpp directly avoids a second daemon and gives us control over context, sampling and grammar-constrained output. We can still let advanced users point BrainWashed at an existing Ollama or LM Studio server.
- **React Native + Expo over Flutter:** the host UI is already React/TS, so the API client, types and much of the chat UI logic are shared. Expo handles iOS/Android builds and OTA updates.
- **A relay for remote access:** the host keeps an outgoing WebSocket to a small relay (`crates/relay`), and devices send their already end-to-end encrypted requests through it. No router ports, no third-party VPN, and users can run their own relay. iroh was the first plan, but phones (React Native) and browsers can't speak it without native modules, while the relay works with plain HTTPS. Tailscale/WireGuard stays a documented alternative for people who already use it. See [relay.md](relay.md).

## 2. System diagram

```
  Phone (RN/Expo)                         Laptop (Tauri host)
 ┌───────────────┐   paired, E2E enc.   ┌──────────────────────────────────┐
 │ Chat UI       │◀───────────────────▶ │ Gateway (auth, device keys)      │
 │ Skills mgmt   │  LAN: TLS, pinned    │   │                              │
 │ Host picker   │  Away: relay (HTTPS) │   ▼                              │
 └───────────────┘                      │ Orchestrator                     │
                                        │   ├─ Skills engine (.md files)   │
                                        │   ├─ Conversation store (SQLite) │
                                        │   └─ Tool runner (later)         │
                                        │   ▼                              │
                                        │ llama-server (sidecar, GPU/CPU)  │
                                        │ Model manager (HF downloads)     │
                                        └──────────────────────────────────┘
```

The phone never talks to `llama-server` directly. Everything goes through the gateway, which checks the device key, then the orchestrator builds the prompt (system prompt + selected skills + history) and streams tokens back over SSE/WebSocket.

## 3. Model runtime and model manager

- Ship platform builds of `llama-server` per backend: macOS arm64 (Metal), Windows/Linux x64 (CUDA, Vulkan, CPU). On first run, detect hardware (GPU vendor, VRAM, RAM) and pick the backend and a sensible default quantization.
- Model catalog: a curated JSON list (BrainWashed model first, then popular small models) plus "paste any Hugging Face GGUF URL". Resumable downloads with SHA-256 check.
- Hardware tiers to advertise:
  - 8 GB RAM laptop, no GPU: 2-3B at Q4 (the BrainWashed model), ~10-20 tokens/s.
  - 16 GB Apple silicon or 8 GB VRAM GPU: up to 7-8B at Q4.
  - 32 GB+ / 16 GB+ VRAM: 14B and up.
- Expose a local OpenAI-compatible endpoint (`/v1/chat/completions`) so other apps on the laptop can use the host too.
- Later: MLX backend on Apple silicon for speed, and multiple models loaded at once if memory allows.

## 4. Skills ("teach it with a markdown file")

Important framing: a skill is **injected into the prompt at runtime**, not trained into the weights. That is what makes it instantly extensible: drop a file in, it works on the next message, no retraining.

**Format** (`~/BrainWashed/skills/<name>/SKILL.md`):

```markdown
---
name: meal-planner
description: Plans weekly meals from what's in the fridge and dietary goals.
triggers: [meal plan, what should I cook, groceries]
version: 1
---
When the user asks for a meal plan:
1. Ask for ingredients on hand if not given.
2. ...
```

**Loading, sized for a small model:** a 2-3B model has a limited context and gets confused by long prompts, so we use progressive disclosure:
1. The system prompt always carries only a short index: each skill's `name` + one-line `description`.
2. Per message, a router picks at most 1-2 skills: embedding similarity (e.g. a ~100 MB embedding model run through the same llama.cpp) plus keyword `triggers`.
3. Only the chosen skills' full bodies are added to the prompt for that turn.

**Management:** file watcher hot-reloads skills; host UI and phone app can list, enable/disable, create and edit skills; skills can be shared as a folder or zip. Later: a skill can include small scripts/tools the host runs in a sandbox, behind an explicit permission prompt.

**Optional later:** "bake" a heavily used skill into a LoRA adapter for users who want it without prompt cost. This is an add-on, not the main path.

## 5. Ahmad's fine-tuned 2-3B model

- **Base model:** pick an Apache-2.0 licensed one so the whole project stays cleanly open source (e.g. a Qwen 3 or SmolLM-family small model). Gemma and Llama bases work technically but carry their own usage licenses that downstream users must accept.
- **What to train for:** not general knowledge (the base has that) but the BrainWashed behaviors: following an injected `SKILL.md` precisely, using the skill index to decide what is relevant, clean tool-call JSON, and staying concise on low-end hardware.
- **Data:** synthetic examples of (skill index + selected skill + user message → ideal answer), generated by a larger model and filtered, plus a held-out eval set of skills the model never saw in training.
- **Tooling:** Unsloth or Axolotl with QLoRA on a single 24 GB GPU (or rented), merge, convert with llama.cpp's `convert_hf_to_gguf.py`, quantize to Q4_K_M and Q8_0, publish on Hugging Face.
- **Eval gate before each release:** skill-following accuracy on unseen skills, tool-call validity rate, tokens/s on an 8 GB laptop.

## 6. Phone-to-home connectivity and security

- **Pairing:** host shows a QR code containing its public key, a one-time token and LAN address. Phone scans, both sides store each other's keys. Hosts can revoke a device at any time.
- **At home (Phase 3):** discover via mDNS, connect over TLS with the host's self-signed cert pinned from the QR code.
- **Away (Phase 4):** through a relay addressed by the host's key, which only sees encrypted traffic. Users run their own or use one they trust; direct P2P when the network allows is a later optimization.
- **Defaults that keep users safe:** nothing listens on public interfaces; every request needs a paired device key; rate limiting; no telemetry unless opted in.
- **Laptop asleep:** show "host offline" clearly on the phone; host setting to prevent sleep while plugged in. Wake-on-LAN is a stretch goal.

## 7. Mobile app

The iOS and Android apps are closed source and sold separately as one-time purchases; they live in a private repository. Everything they do goes through the public [client protocol](client-protocol.md), and the free web chat served by the host covers the same features.

- Screens: host list and pairing, chat (streaming, markdown, copy), model picker, skills list/editor, settings.
- Local conversation cache in SQLite; host is the source of truth.
- Shared `@brainwashed/api` TypeScript package (Apache-2.0) used by the web chat and the apps.
- Later: voice input (on-device speech-to-text), share-sheet "ask BrainWashed", optional tiny on-device model for offline use.

## 8. Repository layout (one monorepo)

```
brainwashed/
  apps/host/          Tauri app (src-tauri = Rust core, src = React UI)
  apps/web/           Web chat served by the host
  packages/api/       Shared TS types + client
  crates/skills/      Skill parser, router
  crates/gateway/     Pairing, auth, relay client
  crates/relay/       Relay server for access away from home
  model/              Fine-tune scripts, data gen, evals
  skills-examples/    Starter skills
  docs/
```

License recommendation: Apache-2.0 (patent grant, friendly to contributors and companies).

## 9. Phased build plan

| Phase | Outcome | Done when |
|---|---|---|
| **0. Foundations** | Monorepo, CI building host on all 3 OSes, license, contributing guide | Empty Tauri app builds and runs on macOS, Windows, Linux in CI |
| **1. Desktop MVP** | Download a model, run it, chat with it on the laptop | Fresh install to first answer in under 5 minutes on an 8 GB laptop |
| **2. Skills v1** | `.md` skills load, route and hot-reload | Example skills reliably change behavior with a stock 3B model |
| **3. Mobile on LAN** | Pair phone by QR, chat over home Wi-Fi | iOS and Android builds chatting with the host on the same network |
| **4. Remote access** | Phone works away from home, E2E encrypted | Chat works over cellular with no router configuration |
| **5. BrainWashed model v1** | Fine-tuned 2-3B released as the default model | Beats its base model on the skill-following eval |
| **6. Polish and release** | Signed installers, auto-update, app store listings, docs site | Public 1.0 on GitHub, App Store and Play Store |

Phase 5 (model) can run in parallel with Phases 1-4; the app works with any GGUF model until it lands.

## 10. Risks and costs to plan for

- **Code signing:** Apple Developer account ($99/yr) is needed for macOS notarization and the iOS App Store; Google Play is $25 once; Windows signing certificates cost money or installs show SmartScreen warnings.
- **GPU backend matrix:** CUDA/ROCm/Vulkan builds multiply CI and installer size. Start with Metal + Vulkan + CPU and add CUDA as a separate download.
- **App store review:** apps that connect to "your own AI server" are generally accepted, but the app must be useful on review (provide a demo host or demo mode).
- **Small model quality:** 2-3B models are fragile with long or conflicting instructions, which is why skill routing caps active skills at 1-2.

## 11. Decisions needed from Ahmad

1. GitHub org/repo name and license (recommend Apache-2.0).
2. Base model for the fine-tune (recommend an Apache-2.0 small model).
3. Whether to start Phase 0 now: I can scaffold the monorepo once a repository is connected.
