# Relay: using BrainWashed away from home

At home, devices talk to your computer directly on the local network. Away from home they can't: your router blocks incoming connections, and your computer's address changes. The relay solves this with no router setup.

## How it works

1. Your computer opens an outgoing WebSocket to the relay (`/host/connect`) and keeps it open while device access is on.
2. The relay sends a random challenge. The computer seals it with its secret key, which proves it owns its public key. Nobody else can register under that key.
3. A paired device sends the same requests it sends at home, to `https://<relay>/h/<host key>/pair`, `/rpc` or `/hello`. The relay passes each one down the WebSocket and streams the answer back.

The relay only forwards those three paths. It does not serve the web chat, because a page delivered through the relay could be altered by whoever runs it. Away from home, use the BrainWashed app.

## What the relay can and can't see

Every request after pairing is already end-to-end encrypted to your computer's key (see [client-protocol.md](client-protocol.md)), and your computer checks every request itself: pairing tokens, device keys, timestamps and replayed nonces. So the relay can't read your chats or send commands as one of your devices.

It can see:

- which computer (by public key) and which paired device (by device ID) are talking,
- when, and how much data,
- the IP addresses of both ends.

It could also refuse to forward traffic. It stores nothing; everything lives in memory and is gone on restart.

## Running a relay

The relay is one small binary with no configuration. Run it behind anything that terminates HTTPS and passes WebSockets through (Caddy, nginx, Fly.io, Render, a Kubernetes ingress).

With Docker, from the repository root:

```sh
docker build -f crates/relay/Dockerfile -t brainwashed-relay .
docker run -d --restart unless-stopped -p 8080:8080 brainwashed-relay
```

Without Docker:

```sh
cargo run --release -p brainwashed-relay -- 0.0.0.0:8080
```

It listens on `$PORT` (default 8080) or the address you pass. `RUST_LOG=debug` shows more.

A Caddy front end with automatic HTTPS:

```
relay.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

Avoid proxies that buffer responses (for example, turn off `proxy_buffering` in nginx), or chat replies arrive all at once instead of streaming.

## Using it

1. On the computer: **Devices**, then **Away from home**. Enter the relay's address, for example `https://relay.example.org`, and save. The status line shows when it's connected.
2. Pair the BrainWashed app as usual, on the same Wi-Fi. The pairing code now includes the relay address.
3. Away from home, the app tries your computer's local addresses first, then the relay.

Devices paired before you set a relay don't know about it. Pair them again.

## Limits

- Each computer can have 64 requests in flight through a relay; more are refused with 429.
- Request bodies are capped at 8 MB.
- A computer that is asleep or has device access off shows as offline (503).
