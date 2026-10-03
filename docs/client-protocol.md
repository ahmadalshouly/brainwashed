# Client protocol (v1)

How clients talk to the desktop host on the local network: the web chat the host serves, the BrainWashed phone apps, and anything else someone builds. Implemented in `crates/gateway` (host) and `packages/api/src/remote.ts` (TypeScript client, used by the web chat and the apps).

## Stability

This protocol is a public, versioned interface. The web chat and the official apps use only what is documented here, with no private endpoints, so anything they can do, any client can.

- `v` in the pairing link and `protocol` from `GET /hello` give the protocol version, currently `1`.
- Within a version, changes are additive only: new methods, new optional params, new fields in results and new chat event kinds. Clients must ignore fields and event kinds they don't know.
- Removing or changing the meaning of anything is a new version. The host keeps serving the previous version for at least one release after a new one ships.

## Keys

- The host has a long-lived X25519 key pair, stored at `<data dir>/gateway/host.key` (mode 0600).
- Each client makes a new X25519 key pair when it pairs with a host and keeps it private: the apps in the platform keychain (Expo SecureStore), the web chat in `localStorage`.
- Every message is a NaCl `crypto_box`: X25519, then XSalsa20-Poly1305, with a random 24-byte nonce. The host uses the `crypto_box` crate and the TypeScript client uses `tweetnacl`.

## Pairing

1. The user turns on device access and clicks **Pair a device**. The host has two forms of the same link:
   - App link: `brainwashed://pair?v=1&k=<host public key>&t=<one-time token>&a=<LAN IPs>&p=<port>&n=<host name>`
   - Web link: `http://<LAN IP>:<port>/#pair?v=1&k=…` with the same fields after `#pair?`.

   The QR code shows the web link, so a phone camera opens the web chat. Clients must accept both forms.
2. The client sends `POST /pair` with `{ devicePublicKey, n, c }`. Here `c` is `{ token, deviceName }`, encrypted to the host key from the QR code.
3. The host decrypts the request and consumes the token. A token works once and expires after 10 minutes. The host then stores the device and replies with `{ deviceId, hostId, hostName }`, encrypted to the client's key.

The host's public key reaches the client only through the QR code (or the page the host served, for the web chat), so a machine on the network can't pose as the host. Without the host's private key it can't read the token or forge replies.

## Calls

`POST /rpc` with `{ deviceId, n, c }`, where `c` decrypts to `{ ts, method, params }`.

The host refuses the call if any of these hold:

- The device is not paired, or has been removed.
- The ciphertext fails to decrypt.
- `ts` is more than 5 minutes from the host's clock.
- The nonce was already used, which stops replay. Nonces are remembered for twice the allowed clock skew.

Replies are `{ n, c }` and decrypt to `{ ok: result }` or `{ error: message }`.

Methods: `info`, `state`, `models`, `loadModel { id }`, `skills`, `setSkillEnabled { name, enabled }`, and `chat { messages }`.

`chat` streams `application/x-ndjson`. Each line is an encrypted frame that decrypts to one of:

- `{ event: ChatEvent }`, sent while the reply streams. The first event is always the `skills` event.
- `{ done: answer }` or `{ error }`, sent once at the end.

## Web chat

`GET /` and every path except `/hello`, `/pair` and `/rpc` serve the web chat (`apps/web`), a single-page app built into the host.

- The pairing details sit in the URL fragment, which browsers never send over the network. The page pairs, then removes the fragment from the address bar and history.
- A browser can only call the computer that served the page, so it uses the page's own address and ignores `a` and `p`.
- The browser keeps its key pair in `localStorage`. The page is served with a strict Content Security Policy (`connect-src 'self'`, no framing) so other sites can't read it.
- Limit: the page itself arrives over plain HTTP, so someone who can tamper with traffic on your network could serve a modified page. The apps don't have this weakness, because their code doesn't come from the network. HTTPS with a pinned certificate is planned with remote access.

## Not covered yet

- **Remote access away from home:** Phase 4, a peer-to-peer connection with relay fallback.
- **Automatic discovery when the computer's IP address changes:** for now, the app tries every address from the QR code and remembers the last one that worked. If all of them fail, pair again.
- **Forward secrecy:** a stolen host key would expose recorded traffic. Moving to ephemeral session keys is planned alongside Phase 4.
