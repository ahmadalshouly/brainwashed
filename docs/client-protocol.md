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
   - App link: `brainwashed://pair?v=1&k=<host public key>&t=<one-time token>&a=<LAN IPs>&p=<port>&n=<host name>`, plus `&u=<public URL>` when the host is reachable from anywhere (a tunnel or the owner's own domain), and `&r=<relay URL>` when it uses a relay.
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

Any paired device can call `info`, `state`, `models`, `skills`, `whoami` (its `{ deviceId, name, role }`) and `chat { messages }`.

Admins can also call:

| Area | Methods |
|---|---|
| Models | `loadModel { id }`, `unloadModel`, `hardware`, `catalog`, `downloadModel { repo, quant? }` (returns at once; poll `downloads`), `downloads`, `deleteModel { id }` |
| Skills | `setSkillEnabled { name, enabled }`, `skillList` (folder, skills, load errors), `skillSource { name }`, `saveSkill { source, previousName? }`, `deleteSkill { name }` |
| Settings | `settings`, `updateSettings { settings }` (only the fields given change; the tunnel token is never sent back, only `tunnel_token_set`), `access` (addresses, tunnel, relay, public URL), `checkForUpdate` |
| Devices | `devices`, `createPairingOffer { role }` (includes `qr`, the code as rows of `0`/`1`), `removeDevice { id }`, `setDeviceRole { id, role }`, `renameDevice { id, name }` |
| Audit | `auditLog { limit? }`, newest first |

A member calling an admin method gets `{ error }`. Devices paired before roles existed are admins. Every admin call that changes something, and every pairing, is written to the host's audit log.

`chat` streams `application/x-ndjson`. Each line is an encrypted frame that decrypts to one of:

- `{ event: ChatEvent }`, sent while the reply streams. The first event is always the `skills` event.
- `{ done: answer }` or `{ error }`, sent once at the end.

## Public address

When the pairing link has `u`, the host is reachable at that `https://` URL from anywhere, with the same paths (`/hello`, `/pair`, `/rpc`) and the same bodies. Clients should try the local addresses first, then `u`, then the relay, and remember whichever answered. A quick tunnel's address changes when the host restarts; clients that can't reach any address should ask the user to pair again.

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
