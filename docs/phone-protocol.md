# Phone protocol

How the phone app talks to the desktop host on the local network. Implemented in `crates/gateway` (host) and `packages/api/src/remote.ts` (phone).

## Keys

- The host has a long-lived X25519 key pair, stored at `<data dir>/gateway/host.key` (mode 0600).
- Each phone makes a new X25519 key pair when it pairs with a host and keeps it in the platform keychain (Expo SecureStore).
- Every message is a NaCl `crypto_box`: X25519, then XSalsa20-Poly1305, with a random 24-byte nonce. The host uses the `crypto_box` crate and the phone uses `tweetnacl`.

## Pairing

1. The user turns on phone access and clicks **Pair a phone**. The host shows a QR code containing:
   `brainwashed://pair?v=1&k=<host public key>&t=<one-time token>&a=<LAN IPs>&p=<port>&n=<host name>`
2. The phone sends `POST /pair` with `{ devicePublicKey, n, c }`. Here `c` is `{ token, deviceName }`, encrypted to the host key from the QR code.
3. The host decrypts the request and consumes the token. A token works once and expires after 10 minutes. The host then stores the device and replies with `{ deviceId, hostId, hostName }`, encrypted to the phone's key.

The host's public key reaches the phone only through the QR code, so a machine on the network can't pose as the host. Without the host's private key it can't read the token or forge replies.

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

## Not covered yet

- **Remote access away from home:** Phase 4, a peer-to-peer connection with relay fallback.
- **Automatic discovery when the computer's IP address changes:** for now, the phone tries every address from the QR code and remembers the last one that worked. If all of them fail, pair again.
- **Forward secrecy:** a stolen host key would expose recorded traffic. Moving to ephemeral session keys is planned alongside Phase 4.
