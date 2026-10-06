import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import worker from "../src/index.js";
import { cleanUrl, idOf, KEEP_SECONDS } from "../src/book.js";

/** Enough of Cloudflare D1 for the worker, on Node's SQLite. */
function fakeD1() {
  const db = new DatabaseSync(":memory:");
  db.exec(readFileSync(new URL("../schema.sql", import.meta.url), "utf8"));
  return {
    raw: db,
    prepare(sql) {
      return {
        bind: (...args) => ({
          first: async () => db.prepare(sql).get(...args) ?? null,
          run: async () => db.prepare(sql).run(...args),
        }),
      };
    },
  };
}

const KEY = "s3cr3t-key-for-tests-0123456789abcdefghijklmnopqrstuv";
const OTHER = "another-key-for-tests-0123456789abcdefghijklmnopqrstu";

function call(env, method, path, body) {
  return worker.fetch(
    new Request(`https://book.example${path}`, {
      method,
      ...(body ? { body: JSON.stringify(body), headers: { "Content-Type": "application/json" } } : {}),
    }),
    env,
  );
}

test("a computer saves its address and devices look it up", async () => {
  const env = { DB: fakeD1() };
  const id = await idOf(KEY);
  assert.match(id, /^[A-Za-z0-9_-]{43}$/);

  assert.equal((await call(env, "GET", `/a/${id}`)).status, 404);
  const put = await call(env, "PUT", `/a/${id}`, { key: KEY, url: "https://quiet-river.trycloudflare.com/" });
  assert.equal(put.status, 200);

  const got = await call(env, "GET", `/a/${id}`);
  assert.equal(got.status, 200);
  assert.equal(got.headers.get("access-control-allow-origin"), "*");
  const entry = await got.json();
  assert.equal(entry.url, "https://quiet-river.trycloudflare.com");
  assert.ok(Math.abs(entry.updated - Date.now() / 1000) < 5);

  // A restart gets a new address; the same id now points there.
  await call(env, "PUT", `/a/${id}`, { key: KEY, url: "https://brave-moon.trycloudflare.com" });
  assert.equal((await (await call(env, "GET", `/a/${id}`)).json()).url, "https://brave-moon.trycloudflare.com");

  // Browsers can bookmark a link that always goes to the current address.
  const go = await call(env, "GET", `/go/${id}`);
  assert.equal(go.status, 302);
  assert.equal(go.headers.get("location"), "https://brave-moon.trycloudflare.com/");
});

test("only the computer with the key can change or remove its entry", async () => {
  const env = { DB: fakeD1() };
  const id = await idOf(KEY);
  await call(env, "PUT", `/a/${id}`, { key: KEY, url: "https://mine.trycloudflare.com" });

  const hijack = await call(env, "PUT", `/a/${id}`, { key: OTHER, url: "https://evil.example" });
  assert.equal(hijack.status, 403);
  assert.equal((await call(env, "PUT", `/a/${id}`, { url: "https://evil.example" })).status, 400);
  assert.equal((await call(env, "DELETE", `/a/${id}`, { key: OTHER })).status, 403);
  assert.equal((await (await call(env, "GET", `/a/${id}`)).json()).url, "https://mine.trycloudflare.com");

  assert.equal((await call(env, "DELETE", `/a/${id}`, { key: KEY })).status, 200);
  assert.equal((await call(env, "GET", `/a/${id}`)).status, 404);
  assert.equal((await call(env, "GET", `/go/${id}`)).status, 404);
});

test("rejects addresses that aren't a plain https origin", async () => {
  const env = { DB: fakeD1() };
  const id = await idOf(KEY);
  for (const url of [
    "http://plain.example",
    "https://user:pw@x.example",
    "https://x.example/path",
    "https://x.example/?q=1",
    "javascript:alert(1)",
    `https://${"a".repeat(200)}.example`,
    42,
  ]) {
    assert.equal((await call(env, "PUT", `/a/${id}`, { key: KEY, url })).status, 400, String(url));
  }
  assert.equal(cleanUrl("https://ai.example.com:8443"), "https://ai.example.com:8443");
});

test("ignores bad ids and unknown routes", async () => {
  const env = { DB: fakeD1() };
  assert.equal((await call(env, "GET", "/a/short")).status, 404);
  assert.equal((await call(env, "GET", "/a/" + "x".repeat(43) + "/more")).status, 404);
  assert.equal((await call(env, "GET", "/nothing")).status, 404);
  assert.equal((await call(env, "POST", `/a/${await idOf(KEY)}`, { key: KEY })).status, 405);
  assert.equal((await call(env, "PUT", `/a/${await idOf(KEY)}`, null)).status, 400);
  assert.equal((await call(env, "OPTIONS", `/a/${await idOf(KEY)}`)).status, 204);
  assert.equal((await call(env, "GET", "/")).status, 200);
});

test("forgets computers that stopped checking in", async () => {
  const env = { DB: fakeD1() };
  const id = await idOf(KEY);
  await call(env, "PUT", `/a/${id}`, { key: KEY, url: "https://old.trycloudflare.com" });
  env.DB.raw.prepare("UPDATE addresses SET updated = ?").run(Math.floor(Date.now() / 1000) - KEEP_SECONDS - 1);
  assert.equal((await call(env, "GET", `/a/${id}`)).status, 404);
  await worker.scheduled({}, env);
  assert.equal(env.DB.raw.prepare("SELECT COUNT(*) AS n FROM addresses").get().n, 0);
});
