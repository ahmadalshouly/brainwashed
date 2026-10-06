# Address book

A tiny Cloudflare Worker that lets paired devices find a BrainWashed computer after its address changes.

The free Cloudflare quick tunnel gives the computer a new random `https://<words>.trycloudflare.com` address every time BrainWashed starts. While BrainWashed runs, it posts its current address here under an id only it can write to. Pairing codes carry the lookup link. When a phone can't reach the computer at the address it saved, it asks the address book for the new one and carries on, so nobody has to scan a new code after a restart.

- **What it stores:** the id, the current address, and when it was last updated. It never sees chats. Calls between devices and the computer stay end-to-end encrypted to the computer's key, so even a wrong answer from the address book can't make a device talk to anyone else.
- **Who can change an entry:** the id is the SHA-256 of a secret kept on the computer (`gateway/address.key`), so only that computer can change or remove its entry.
- **Forgetting:** the computer re-posts every 12 hours while it runs. Entries not refreshed in 30 days are deleted by a daily job.
- **New address:** **Remote access > New address** in the admin page picks a new secret and removes the old entry, so links shared earlier stop working. It also starts a new free tunnel address.

## Routes

| Route | Does |
|---|---|
| `GET /a/<id>` | `{"url": "https://…", "updated": <unix seconds>}`, or 404 |
| `PUT /a/<id>` with `{"key": "<secret>", "url": "https://…"}` | Saves the address. The URL must be `https://` with nothing after the host name. |
| `DELETE /a/<id>` with `{"key": "<secret>"}` | Removes the entry |
| `GET /go/<id>` | Redirects to the current address. A bookmark that always opens the computer's web page. |

## Deploy

It fits in Cloudflare's free plan (Workers and D1).

```sh
cd services/address-book
npx wrangler login
npx wrangler d1 create brainwashed-address-book   # copy the database_id it prints into wrangler.toml
npx wrangler d1 execute brainwashed-address-book --remote --file schema.sql
npx wrangler deploy
```

`wrangler deploy` prints the address, like `https://brainwashed-address-book.<you>.workers.dev`. You can also add a custom domain to the Worker in the Cloudflare dashboard.

To make BrainWashed use it:

- **For everyone:** set `DEFAULT_ADDRESS_BOOK` in `crates/gateway/src/address_book.rs` and release.
- **For one computer:** `brainwashed remote book https://your-address-book`, or **Remote access > Address book** in the admin page. `brainwashed remote book off` turns it off.

## Test

```sh
node --test test/index.test.js
```

The tests run the Worker against Node's built-in SQLite in place of D1. `npx wrangler dev --local` runs it in Cloudflare's real runtime (run the `d1 execute` line with `--local` first).
