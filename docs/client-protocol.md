# Client protocol (v1)

How clients talk to the BrainWashed host (the `brainwashed` command), at home or from anywhere: the web app the host serves, the BrainWashed phone apps, and anything else someone builds. Implemented in `crates/gateway` (host) and `packages/api/src/remote.ts` (TypeScript client, used by the web app and the apps).

## Stability

This protocol is a public, versioned interface. The web app and the official apps use only what is documented here, with no private endpoints, so anything they can do, any client can.

- `v` in the pairing link and `protocol` from `GET /hello` give the protocol version, currently `1`.
- Within a version, changes are additive only: new methods, new optional params, new fields in results and new chat event kinds. Clients must ignore fields and event kinds they don't know.
- Removing or changing the meaning of anything is a new version. The host keeps serving the previous version for at least one release after a new one ships.

## Keys

- The host has a long-lived X25519 key pair, stored at `<data dir>/gateway/host.key` (mode 0600).
- Each client makes a new X25519 key pair when it pairs with a host and keeps it private: the apps in the platform keychain (Expo SecureStore), the web app in `localStorage`.
- Every message is a NaCl `crypto_box`: X25519, then XSalsa20-Poly1305, with a random 24-byte nonce. The host uses the `crypto_box` crate and the TypeScript client uses `tweetnacl`.

## Pairing

1. An admin clicks **Show code** under **Devices**, choosing whether the new device is an admin or a member. The host has two forms of the same link:
   - App link: `brainwashed://pair?v=1&k=<host public key>&t=<one-time token>&a=<LAN IPs>&p=<port>&n=<host name>`, plus `&u=<public URL>` when the host is reachable from anywhere (a tunnel or the owner's own domain), and `&r=<relay URL>` when it uses a relay. `&b=<lookup URL>` says where to look the public address up after it changes (see below).
   - Web link: `<public URL>/#pair?v=1&k=…`, or `http://<LAN IP>:<port>/#pair?v=1&k=…` with no public URL, with the same fields after `#pair?`.

   The QR code shows the web link, so a phone camera opens the web app. Clients must accept both forms.
2. The client sends `POST /pair` with `{ devicePublicKey, n, c }`. Here `c` is `{ token, deviceName }`, encrypted to the host key from the QR code.
3. The host decrypts the request and consumes the token. A token works once and expires after 10 minutes. The host then stores the device with the token's role and replies with `{ deviceId, hostId, hostName, role }`, encrypted to the client's key. `role` is `admin` or `member`; hosts before roles existed leave it out.

The host's public key reaches the client only through the QR code (or the page the host served, for the web app), so a machine on the network can't pose as the host. Without the host's private key it can't read the token or forge replies.

## Calls

`POST /rpc` with `{ deviceId, n, c }`, where `c` decrypts to `{ ts, method, params }`.

The host refuses the call if any of these hold:

- The device is not paired, or has been removed.
- The ciphertext fails to decrypt.
- `ts` is more than 5 minutes from the host's clock.
- The nonce was already used, which stops replay. Nonces are remembered for twice the allowed clock skew.

Replies are `{ n, c }` and decrypt to `{ ok: result }` or `{ error: message }`.

Any paired device can call `info`, `state`, `models`, `skills`, `whoami` (its `{ deviceId, name, role }`), `chatModels`, `chatDefaults`, `readDocument { name, data }`, `chat { messages, options?, model?, replyId? }`, `chatResume { replyId, after }` and `chatStop { replyId }`.

- `chatModels` lists what the chat can use: `{ id, name, provider, vision, cloud }`. `local` is the model running on the computer (listed only while one is loaded); cloud models are `<provider>/<model>` and appear when an admin connects a provider. Members see only the models admins share with them.
- `chatDefaults` returns the admin's model settings for every chat, in the same shape as chat `options`. The host already applies them to anything a chat leaves out; clients show them so people know what "default" means.
- `readDocument` takes a file as base64 (up to 25 MB) and returns `{ text, pages?, truncated }`. It reads PDFs, Word (.docx) and UTF-8 text. Send the text back as a `file` attachment.

Admins can also call:

| Area | Methods |
|---|---|
| Models | `loadModel { id }`, `unloadModel`, `hardware`, `catalog`, `downloadModel { repo, quant? }` (returns at once; poll `downloads`), `downloads`, `deleteModel { id }`, `setModelSpeedup { id, speedup: id \| null }` |
| Skills | `setSkillEnabled { name, enabled }`, `skillList` (folder, skills, load errors), `skillSource { name }`, `saveSkill { source, previousName? }`, `deleteSkill { name }`, `communitySkills` (the community index), `previewSkill { spec }` (a community skill name or a link; returns the source, its SHA-256 and warnings), `installSkill { spec, sha256, replace? }` |
| Settings | `settings`, `updateSettings { settings }` (only the fields given change; the tunnel token is never sent back, only `tunnel_token_set`), `access` (addresses, tunnel, relay, public URL), `checkForUpdate`, `newAddress` (new address book id and tunnel address; returns `access`) |
| Devices | `devices`, `createPairingOffer { role }` (includes `qr`, the code as rows of `0`/`1`), `removeDevice { id }`, `setDeviceRole { id, role }`, `renameDevice { id, name }` |
| Cloud providers | `providers` (keys are never sent back, only `keySet` and `keyHint`), `saveProvider { provider: { id, name, baseUrl, apiKey?, models, members } }` (no `apiKey` keeps the saved key), `deleteProvider { id }`, `providerModels { baseUrl, apiKey?, id? }` |
| Tools (MCP servers) | `mcpServers` (each with `status`, `tools`, `serverInfo` and the last lines of `log`), `saveMcpServer { server: { id, name, enabled, members, transport } }` where `transport` is `{ type: "stdio", command, args, env, cwd? }` or `{ type: "http", url, headers }` (starts it in the background; poll `mcpServers` while its status is `starting`), `setMcpServerEnabled { id, enabled }`, `restartMcpServer { id }`, `deleteMcpServer { id }` |
| API keys | `apiKeys`, `createApiKey { name, role? }` (returns `{ key, secret }`; the secret is shown only this once), `revokeApiKey { id }`. The keys are for the OpenAI-compatible API in [api.md](api.md). |
| Audit | `auditLog { limit? }`, newest first |

Speculative decoding drafts (DFlash, DSpark, EAGLE-3, Gemma 4 assistant GGUFs) are listed with `InstalledModel.draft` set to their llama-server `--spec-type`. They can't be loaded alone (`loadModel` refuses them with an explanation); `setModelSpeedup` pairs one with the main model it was made for, which then runs with `--model-draft`. Changing it reloads the model if it is running.

A member calling an admin method gets `{ error }`. Devices paired before roles existed are admins. Every admin call that changes something, and every pairing, is written to the host's audit log.

### Chat

`chat` takes:

- `messages`: `{ role, content, attachments?, toolCalls?, toolCallId? }[]`. An attachment is `{ type: "image", name, mime, data }` (base64 PNG, JPEG, WebP, GIF or BMP) or `{ type: "file", name, text }`. Pictures reach the model only if it can see them: the local model when it has a vision projector (`InstalledModel.mmproj`), or a cloud model. A model that can't see gets `[Picture: name]` instead, and the call fails if the latest user message has pictures. Requests can be up to 48 MB. To keep what tools returned in the history, send an assistant message with `toolCalls` (the calls, as `tool_call` events gave them), then a `{ role: "tool", toolCallId, content }` message per result, then the assistant's answer. Models that aren't offered tools get the answers without the calls.
- `options` (all optional): `temperature` (0 to 2), `topP`, `topK`, `minP`, `repeatPenalty`, `presencePenalty`, `seed`, `maxTokens`, and `reasoning` (true or false turns thinking on or off for models whose template supports it, such as Qwen3). Anything left out uses the admin's `chatDefaults` (the `chat_defaults` setting), then the model's own default. Cloud providers get only the OpenAI-standard fields.
- `model`: an id from `chatModels`. Left out, the local model answers.
- `replyId`: an id the device picks for this reply (letters, digits, `-` and `_`, up to 64). The host keeps the reply for 15 minutes after it ends, so a device that lost the connection mid-answer can fetch the rest with `chatResume`. With an id, the host keeps writing the answer when the device disconnects. Without one, closing the stream stops the answer.

It streams `application/x-ndjson` (uncompressed: send `Accept-Encoding: identity`; on Android Expo also needs `Accept: text/event-stream` so its dev-build network inspector doesn't hold the body back). Each line is an encrypted frame that decrypts to one of:

- `{ event: ChatEvent }`, sent while the reply streams. The first event is always the `skills` event. Then `reasoning` and `content` pieces, and last a `stats` event: `{ kind: "stats", promptTokens, tokens, tokensPerSecond, truncated }`. When the model calls a tool, the host takes the markup out of `content` and sends `{ kind: "tool_call", id, name, arguments, raw? }` (`id` is `call_0`, `call_1`...).
  - `ask_user` is the one tool the host offers (to local models whose chat template supports tools). Its arguments are always `{ question: string, options: string[] }`, also when the model wrote a similar call of its own (`ask`, `clarify`, positional arguments...). Show the question with the options as buttons; a tap sends the option as an ordinary user message. The reply ends with this call. The question is not in `content` or in `done`, so add it (with the options as `- option` lines) to the assistant message when sending the history back.
  - Tools of the MCP servers an admin added on the Tools page (`mcpServers`, `saveMcpServer`, `setMcpServerEnabled`, `restartMcpServer`, `deleteMcpServer`, admins only) are offered to models that support tools, in admins' chats, and in members' chats for servers shared with them. The host runs each call and sends `{ kind: "tool_result", id, name, server, content, isError }` with the call's `id`; then the model carries on, so `content` events can follow, also after several rounds of calls. Show the call as running until its result arrives.
  - Any other call is one the model made up; nothing runs it. Positional arguments are under `"0"`, `"1"`..., and `name` is empty with `raw` set when the call couldn't be read.

  Clients that don't know `tool_call` or `tool_result` can ignore them. The `done` frame then also carries `toolCalls`, the same calls, and `toolResults` when tools ran, so a stored chat can keep them.
- `{ done: answer }` or `{ error }`, sent once at the end.

`chatResume { replyId, after }` streams the same frames for a kept reply, skipping the first `after` (the frames the device already handled), then follows the reply live until it ends. Only the device that started the reply can resume it. If the host no longer has it, the stream is a single `{ error }`.

`chatStop { replyId }` stops a reply this device started, for a Stop button. The model stops generating, and the reply's stream ends with `{ "done": "<text so far>", "stopped": true }` (plus `toolCalls` if there were any). It returns `{ ok: true }`, also when the reply already ended or is unknown.

## Public address

When the pairing link has `u`, the host is reachable at that `https://` URL from anywhere, with the same paths (`/hello`, `/pair`, `/rpc`) and the same bodies. Clients should try the local addresses first, then `u`, then the relay, and remember whichever answered. A quick tunnel's address changes when the host restarts. When the link has `b` (`https://<address book>/a/<id>`), clients that can't reach any address should `GET` it: it answers `{"url": "https://...", "updated": <unix seconds>}` with the host's current public address (or 404). If that differs from the saved `u`, try it, and save it as the new public address when it answers. The address book can't impersonate the host, because replies are still encrypted with the host's key. Clients that still can't reach the host, or have no `b`, should ask the user to pair again. Admins can call `newAddress` to give the host a new lookup id and tunnel address; earlier lookups then return 404.

## Relay

When the pairing link has `r`, the host also accepts the same requests through that relay, at `<r>/h/<k>/hello`, `<r>/h/<k>/pair` and `<r>/h/<k>/rpc`, where `k` is the host key exactly as it appears in the link. Bodies, replies and streaming are unchanged. Clients should try the local addresses first and the relay last, and remember whichever answered.

The relay answers `503` with `{ error }` when the computer isn't connected to it, and `429` when too many requests are in flight. [relay.md](relay.md) describes how the relay works and what it can see.

## Web app

`GET /` and every path except `/hello`, `/pair`, `/rpc` and `/control/*` serve the web app (`apps/web`), a single-page app built into the host: chat for everyone, and admin pages for admins.

`/control/*` is for the `brainwashed` command on the same computer (`open`, `status`, `stop`). It needs a random token the host writes to `<data dir>/gateway/control.token`, readable only by the user running it, and isn't part of this protocol.

- The pairing details sit in the URL fragment, which browsers never send over the network. The page pairs, then removes the fragment from the address bar and history.
- A browser can only call the computer that served the page, so it uses the page's own address and ignores `a` and `p`.
- The browser keeps its key pair in `localStorage`. The page is served with a strict Content Security Policy (`connect-src 'self'`, no framing) so other sites can't read it.
- Limit: on the local network the page arrives over plain HTTP, and through a tunnel the tunnel provider delivers it, so someone who can tamper with that path could serve a modified page. The apps don't have this weakness, because their code doesn't come from the network.

## Not covered yet

- **Direct connections away from home:** traffic away from home always goes through the tunnel or the relay. A direct peer-to-peer path when the network allows it would save a hop.
- **Automatic discovery when the computer's IP address changes:** for now, the app tries every address from the QR code and remembers the last one that worked. If all of them fail, pair again.
- **Forward secrecy:** a stolen host key would expose recorded traffic, including traffic recorded at a relay. Ephemeral session keys are planned.
