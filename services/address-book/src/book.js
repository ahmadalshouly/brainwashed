// BrainWashed address book: a tiny Cloudflare Worker that remembers where each
// computer can be reached right now.
//
// A free Cloudflare quick tunnel gets a new random address every time it
// starts. The computer posts that address here under an id it chose; paired
// devices look it up by that id when the address they saved stops answering,
// so they never need a new pairing code just because BrainWashed restarted.
//
// - The id is the SHA-256 of a secret only the computer knows, so only it can
//   change or remove its entry.
// - Only addresses are stored, never chats. Calls between devices and the
//   computer stay end-to-end encrypted to the computer's key, so even a wrong
//   address can't make a device talk to anyone else.
// - Entries no one has refreshed in 30 days are deleted.
//
// Routes:
//   GET    /a/<id>  -> {"url": "...", "updated": <unix seconds>}, or 404
//   PUT    /a/<id>  {"key": "<secret>", "url": "https://..."}
//   DELETE /a/<id>  {"key": "<secret>"}
//   GET    /go/<id> -> redirect to the address, for a browser bookmark

const ID = /^[A-Za-z0-9_-]{43}$/;
const KEY = /^[A-Za-z0-9_-]{43,128}$/;
const MAX_URL = 200;
export const KEEP_SECONDS = 30 * 24 * 3600;

const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, PUT, DELETE, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
  "Access-Control-Max-Age": "86400",
};

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", "Cache-Control": "no-store", ...CORS },
  });
}

export const fail = (status, error) => json({ error }, status);

function base64url(bytes) {
  let s = "";
  for (const b of new Uint8Array(bytes)) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** The id that belongs to a secret: base64url(SHA-256(secret)). */
export async function idOf(key) {
  return base64url(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(key)));
}

/** An https origin (no path, query or credentials), or null. */
export function cleanUrl(value) {
  if (typeof value !== "string" || value.length > MAX_URL) return null;
  let url;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" || url.username || url.password) return null;
  if ((url.pathname !== "/" && url.pathname !== "") || url.search || url.hash) return null;
  return url.origin;
}

async function body(request) {
  try {
    const value = await request.json();
    return value && typeof value === "object" ? value : null;
  } catch {
    return null;
  }
}

/** Checks the secret in the request matches the id. */
async function owner(request, id) {
  const data = await body(request);
  if (!data || typeof data.key !== "string" || !KEY.test(data.key)) return { error: fail(400, "Send the key.") };
  if ((await idOf(data.key)) !== id) return { error: fail(403, "That key doesn't match this id.") };
  return { data };
}

const now = () => Math.floor(Date.now() / 1000);

async function lookup(db, id) {
  return db.prepare("SELECT url, updated FROM addresses WHERE id = ?1 AND updated > ?2").bind(id, now() - KEEP_SECONDS).first();
}

export async function handle(request, env) {
  const { pathname } = new URL(request.url);
  if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: CORS });
  const match = /^\/(a|go)\/([^/]+)$/.exec(pathname);
  if (!match) {
    if (pathname === "/" && request.method === "GET") {
      return new Response("BrainWashed address book. See https://github.com/ahmadalshouly/brainwashed\n", {
        headers: { "Content-Type": "text/plain; charset=utf-8" },
      });
    }
    return fail(404, "Not found.");
  }
  const [, route, id] = match;
  if (!ID.test(id)) return fail(404, "Not found.");
  const db = env.DB;

  if (route === "go") {
    if (request.method !== "GET") return fail(405, "Use GET.");
    const row = await lookup(db, id);
    if (!row) {
      return new Response("This computer hasn't said where it is lately. Check that BrainWashed is running on it.\n", {
        status: 404,
        headers: { "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" },
      });
    }
    return new Response(null, { status: 302, headers: { Location: `${row.url}/`, "Cache-Control": "no-store" } });
  }

  switch (request.method) {
    case "GET": {
      const row = await lookup(db, id);
      return row ? json({ url: row.url, updated: row.updated }) : fail(404, "Not found.");
    }
    case "PUT": {
      const { data, error } = await owner(request, id);
      if (error) return error;
      const url = cleanUrl(data.url);
      if (!url) return fail(400, "The address must be an https:// address with nothing after the host name.");
      await db
        .prepare(
          "INSERT INTO addresses (id, url, updated) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET url = excluded.url, updated = excluded.updated",
        )
        .bind(id, url, now())
        .run();
      return json({ ok: true });
    }
    case "DELETE": {
      const { error } = await owner(request, id);
      if (error) return error;
      await db.prepare("DELETE FROM addresses WHERE id = ?1").bind(id).run();
      return json({ ok: true });
    }
    default:
      return fail(405, "Use GET, PUT or DELETE.");
  }
}

/** Daily: forget computers that haven't checked in for 30 days. */
export async function forgetStale(env) {
  await env.DB.prepare("DELETE FROM addresses WHERE updated <= ?1").bind(now() - KEEP_SECONDS).run();
}
