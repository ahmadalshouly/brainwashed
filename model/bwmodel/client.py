"""A tiny client for any OpenAI-compatible chat endpoint (llama-server, vLLM,
or a hosted API). Standard library only, so it runs anywhere."""

import json
import time
import urllib.error
import urllib.request


class ChatClient:
    def __init__(self, base_url, model, api_key=None, timeout=300, retries=4, extra_body=None):
        self.url = base_url.rstrip("/") + "/chat/completions"
        self.model = model
        self.api_key = api_key
        self.timeout = timeout
        self.retries = retries
        self.extra_body = extra_body or {}

    def chat(self, messages, tools=None, temperature=0.7, max_tokens=1024, json_mode=False):
        """Returns {"content": str, "tool_calls": [{"name", "arguments"}]}."""
        body = {
            "model": self.model,
            "messages": [to_api_message(m) for m in messages],
            "temperature": temperature,
            "max_tokens": max_tokens,
            **self.extra_body,
        }
        if tools:
            body["tools"] = tools
        if json_mode:
            body["response_format"] = {"type": "json_object"}
        data = self._post(body)
        message = data["choices"][0]["message"]
        calls = []
        for call in message.get("tool_calls") or []:
            fn = call.get("function") or {}
            calls.append({"name": fn.get("name"), "arguments": fn.get("arguments") or "{}"})
        return {"content": message.get("content") or "", "tool_calls": calls}

    def _post(self, body):
        headers = {"Content-Type": "application/json"}
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"
        payload = json.dumps(body).encode()
        for attempt in range(self.retries + 1):
            request = urllib.request.Request(self.url, data=payload, headers=headers)
            try:
                with urllib.request.urlopen(request, timeout=self.timeout) as response:
                    return json.loads(response.read())
            except urllib.error.HTTPError as e:
                retryable = e.code == 429 or e.code >= 500
                if not retryable or attempt == self.retries:
                    detail = e.read().decode(errors="replace")[:500]
                    raise RuntimeError(f"HTTP {e.code} from {self.url}: {detail}") from e
            except (urllib.error.URLError, TimeoutError, ConnectionError):
                if attempt == self.retries:
                    raise
            time.sleep(2 ** (attempt + 1))


def to_api_message(message):
    """Converts a training-format message to the OpenAI wire format, where tool
    call arguments are JSON strings and every call has an id."""
    out = {"role": message["role"], "content": message.get("content") or ""}
    if message.get("tool_calls"):
        out["tool_calls"] = [
            {
                "id": f"call_{i}",
                "type": "function",
                "function": {
                    "name": c["function"]["name"],
                    "arguments": json.dumps(c["function"]["arguments"], ensure_ascii=False),
                },
            }
            for i, c in enumerate(message["tool_calls"])
        ]
    if message["role"] == "tool":
        out["tool_call_id"] = message.get("tool_call_id", "call_0")
        out["name"] = message.get("name")
    return out


def assistant_tool_message(calls):
    """Training-format assistant message for tool calls with dict arguments."""
    return {
        "role": "assistant",
        "content": "",
        "tool_calls": [
            {"type": "function", "function": {"name": c["name"], "arguments": c["arguments"]}}
            for c in calls
        ],
    }


def parse_json_reply(text):
    """Pulls the first JSON object out of a model reply (tolerates code fences)."""
    start, end = text.find("{"), text.rfind("}")
    if start < 0 or end < start:
        raise ValueError(f"no JSON object in reply: {text[:200]!r}")
    return json.loads(text[start : end + 1])
