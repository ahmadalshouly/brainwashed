# OpenAI-compatible API

BrainWashed serves an API in the OpenAI format, so scripts, editors, agents and apps that work with OpenAI can use the models on your computer instead. Replies get the same skills and the same chat defaults (thinking, temperature, reply length) the admin set for the chat.

## Get a key and the address

1. On the admin page, open **API**.
2. Under **Keys**, give the key a name and click **Create key**. Copy it right away: it starts with `bw-` and isn't shown again. Only a hash of it is stored.
3. Copy an address from **Address** and use it as the base URL:
   - `http://localhost:47860/v1` on the computer itself.
   - `http://<computer's address>:47860/v1` from another device on the same network. This isn't encrypted, so use it only on networks you trust.
   - `https://<your tunnel or domain>/v1` from anywhere, when **Remote access** is on. The self-hosted relay doesn't carry the API; use the tunnel or your own domain.

Make one key per app, so you can revoke it on its own under **API**. A key made for **Members' models** can use the model running on the computer and the cloud models shared with members; **All models** also reaches cloud models only admins use. Creating and revoking keys is recorded on the **Activity** page.

## Calls

| Call | What it does |
|---|---|
| `GET /v1/models` | The models the key can use: `local` for the model running on the computer, and `<provider>/<model>` for cloud models. |
| `POST /v1/chat/completions` | A chat reply, whole or streamed (`"stream": true`). |

Every call needs `Authorization: Bearer <key>`. The API allows calls from web pages on other sites (CORS), since a page needs the key to do anything.

### Chat completions

Request fields:

- `model`: `local`, an id from `/v1/models`, or the local model's own name. Left out, the local model answers.
- `messages`: `system` (or `developer`), `user` and `assistant` messages. `content` is a string, or a list of `{ "type": "text", "text" }` and `{ "type": "image_url", "image_url": { "url": "data:image/png;base64,…" } }` parts. Pictures must be data URLs, and the model has to be able to see them.
- `stream`, and `stream_options.include_usage` for a usage chunk at the end.
- `temperature`, `top_p`, `max_tokens` (or `max_completion_tokens`), `seed`, `presence_penalty`, and llama.cpp's `top_k`, `min_p` and `repeat_penalty`. Anything left out uses the admin's chat defaults, then the model's own.
- `reasoning_effort`: `"none"` turns thinking off for models that think (such as Qwen3); any other value turns it on. `reasoning: true|false` does the same.

Your `system` message is added after BrainWashed's own system prompt and skills; it doesn't replace them.

Not supported: tool and function calls, `n` other than 1, `logprobs`, and audio. Unknown fields are ignored.

Some models are trained to call tools and do so even when none are offered. BrainWashed takes that markup out of the answer: a call that only asks the user a question (`ask_user` and similar) comes back as ordinary text (the question, then each option on a `- ` line), and any other call comes back in `tool_calls` with `finish_reason: "tool_calls"`, so it never shows up as raw tokens. Nothing runs these calls.

Replies follow OpenAI's format. A thinking model's thoughts come in `reasoning_content` (in `message` or in each streamed `delta`), as llama.cpp and DeepSeek do. `finish_reason` is `length` when the reply hit `max_tokens`. Errors look like `{ "error": { "message", "type", "code" } }`, with 401 for a missing or revoked key, 404 for an unknown model, and 400 for a bad request.

## Examples

```sh
curl http://localhost:47860/v1/chat/completions \
  -H "Authorization: Bearer $BRAINWASHED_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model": "local", "messages": [{"role": "user", "content": "Hello!"}]}'
```

```python
from openai import OpenAI

client = OpenAI(base_url="http://localhost:47860/v1", api_key="bw-…")
for chunk in client.chat.completions.create(
    model="local",
    messages=[{"role": "user", "content": "Write a haiku about home servers."}],
    stream=True,
):
    print(chunk.choices[0].delta.content or "", end="", flush=True)
```

```js
import OpenAI from "openai";

const client = new OpenAI({ baseURL: "http://localhost:47860/v1", apiKey: "bw-…" });
const reply = await client.chat.completions.create({
  model: "local",
  messages: [{ role: "user", content: "Hello!" }],
});
console.log(reply.choices[0].message.content);
```

Apps with an "OpenAI-compatible" or "custom OpenAI" setting (for example Open WebUI, Continue, or LibreChat) take the same base URL and key.
