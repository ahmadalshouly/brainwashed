# Remote access: using BrainWashed from anywhere

By default BrainWashed opens a free [Cloudflare quick tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/do-more-with-tunnels/trycloudflare/) when it starts. Your computer makes an outgoing connection to Cloudflare and gets a public `https://<random words>.trycloudflare.com` address. Phones and browsers anywhere use that address, with no router setup, port forwarding or account.

The codes on the **Devices** page carry the address, so a phone paired with one works at home and away.

## Choosing how

Change it on the admin page under **Remote access** (applies at once), or with `brainwashed remote` (applies at the next start).

| Option | Address | Needs | Best for |
|---|---|---|---|
| **Free tunnel** (`brainwashed remote quick`, default) | New random address each start | Nothing | Trying it, personal use |
| **Your own domain with Cloudflare** (`brainwashed remote cloudflare <token> https://ai.example.com`) | Stays the same | A free Cloudflare account and a domain on it | Always-on home or office servers, teams |
| **An address you set up** (`brainwashed remote url https://...`) | Stays the same | Tailscale Funnel, ngrok, or your own reverse proxy pointing at `http://localhost:47860` | People who already run one |
| **Relay** (`brainwashed remote relay https://...`) | Stays the same | A server running the BrainWashed relay ([relay.md](relay.md)) | The phone apps through infrastructure you control |
| **Off** (`brainwashed remote off`) | Local network only | Nothing | Air-gapped setups |

BrainWashed downloads `cloudflared` from Cloudflare's GitHub releases the first time a tunnel starts, checks its SHA-256, and keeps it in the data folder under `bin/`. A `cloudflared` already on your `PATH`, or one set under **Settings**, is used instead.

### Free tunnel addresses change

A quick tunnel's address is new every time BrainWashed (or the tunnel) restarts. While it runs, BrainWashed posts its current address to the **address book** ([services/address-book](../services/address-book/README.md)), a tiny service that only learns addresses. Pairing codes carry a link to look it up, so the phone apps find the new address on their own and never need a new code because of a restart.

- **Browsers** remember their pairing per address, so a browser on the old address needs a new code from **Devices**. **Remote access** shows a permanent link that always opens the current address.
- **New address** (on **Remote access**) gives this computer a new address book entry and a new tunnel address, for example if a link leaked. Devices paired before keep working on your network; away from home they need a new code. To lock out a lost device for good, remove it under **Devices**.
- `brainwashed remote book <https://...>` uses your own address book; `brainwashed remote book off` turns it off.

For an address that never changes at all, use your own domain:

1. In the Cloudflare dashboard, go to **Zero Trust > Networks > Tunnels** and create a tunnel. Choose any connector; you only need its token (the long string after `--token`).
2. Add a **public hostname**, for example `ai.example.com`, with service `http://localhost:47860`.
3. On the admin page, **Remote access > Your own domain with Cloudflare**: paste the token and `https://ai.example.com`, then **Save and apply**.

## Security

- **Every device is paired.** A device needs a one-time code from an admin. Codes expire after 10 minutes and work once. Without a pairing, the public address only serves the sign-in page.
- **Chats are end-to-end encrypted.** After pairing, every request and reply is encrypted between the device's key and this computer's key ([client-protocol.md](client-protocol.md)). Cloudflare, a reverse proxy or a relay sees only ciphertext, and can't act as one of your devices.
- **Roles.** Admins manage models, skills, devices, settings and remote access. Members can only chat. The first browser on the computer is an admin; new devices are members unless you choose otherwise.
- **Audit log.** The **Activity** page lists every pairing and every change an admin made, with the device that made it. It's stored on the computer in `gateway/audit.jsonl`. Chats aren't logged.
- **What a tunnel provider can do.** Cloudflare (or your proxy) delivers the web page to browsers, so it could in principle serve a modified page. If that matters to you, use the phone apps, whose code doesn't come from the network, or your own reverse proxy.
- **Turning it off.** `brainwashed --local-only` skips the tunnel for one run; `brainwashed remote off` turns it off for good. Remove a lost device under **Devices** and it can't connect again.
