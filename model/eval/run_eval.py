"""Scores a model on skills and tools it never saw in training.

Point it at any OpenAI-compatible server. To test exactly what ships, serve the
GGUF with llama-server the way the host does (`--jinja`):

    llama-server -m brainwashed-q4_k_m.gguf --jinja --port 8081 &
    python model/eval/run_eval.py --base-url http://127.0.0.1:8081/v1 --out report.json

Every case is checked by rules (format lines, word limits, the right tool with
the right arguments), not by another model, so scores are repeatable.
"""

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

MODEL_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(MODEL_DIR))

from bwmodel.client import ChatClient, assistant_tool_message  # noqa: E402
from bwmodel.prompt import build_system_prompt  # noqa: E402
from bwmodel.skills import load_skills  # noqa: E402
from bwmodel.tools import check_call, load_tools, tool_name  # noqa: E402

EVAL_DIR = Path(__file__).resolve().parent
RAW_TOOL_CALL = re.compile(r"<tool_call>|^\s*\{\s*\"name\"\s*:", re.M)


def load_fixtures():
    skills = {s.name: s for s in load_skills(EVAL_DIR / "skills", MODEL_DIR.parent / "skills-examples")}
    tools = {tool_name(t): t for t in load_tools(EVAL_DIR / "tools.json")}
    return skills, tools


def norm(value):
    if isinstance(value, list):
        value = " ".join(map(str, value))
    return re.sub(r"\s+", "", str(value)).lower()


def check_reply(reply, expect, offered):
    """Returns a list of failed checks for one model reply."""
    failures = []
    text = reply["content"]
    calls = reply["tool_calls"]
    if "tool" in expect:
        wanted = expect["tool"]
        if wanted is None:
            if calls:
                failures.append(f"called {calls[0]['name']} but no tool was needed")
            if RAW_TOOL_CALL.search(text):
                failures.append("wrote a tool call as text")
        elif not calls:
            reason = "malformed tool call" if RAW_TOOL_CALL.search(text) else "no tool call"
            failures.append(f"{reason}, expected {wanted}")
        else:
            call = calls[0]
            args, problems = check_call(call, offered)
            failures += [f"invalid call: {p}" for p in problems]
            if call["name"] != wanted:
                failures.append(f"called {call['name']}, expected {wanted}")
            elif args is not None:
                for key, value in expect.get("args", {}).items():
                    if key not in args:
                        failures.append(f"missing argument {key}")
                    elif isinstance(value, str) and norm(value) not in norm(args[key]):
                        failures.append(f"argument {key}={args[key]!r}, expected {value!r}")
                    elif not isinstance(value, str) and args[key] != value:
                        failures.append(f"argument {key}={args[key]!r}, expected {value!r}")
    if not calls:
        failures += check_text(text, expect)
    return failures


def check_text(text, expect):
    failures = []
    words = len(text.split())
    if expect.get("max_words") and words > expect["max_words"]:
        failures.append(f"{words} words, limit {expect['max_words']}")
    if expect.get("question") and "?" not in text and "؟" not in text:
        failures.append("did not ask a question")
    for pattern in expect.get("include", []):
        if not re.search(pattern, text, re.I | re.M):
            failures.append(f"missing /{pattern}/")
    for pattern in expect.get("exclude", []):
        if re.search(pattern, text, re.I | re.M):
            failures.append(f"should not match /{pattern}/")
    if "max_bullets" in expect:
        bullets = len(re.findall(r"^\s*[-*•]\s+", text, re.M))
        if bullets > expect["max_bullets"]:
            failures.append(f"{bullets} bullets, limit {expect['max_bullets']}")
    if "json_keys" in expect:
        failures += check_json(text, expect)
    return failures


def check_json(text, expect):
    match = re.search(r"```json\s*(.*?)```", text, re.S)
    if not match:
        return ["no ```json block"]
    try:
        data = json.loads(match.group(1))
    except json.JSONDecodeError as e:
        return [f"invalid JSON: {e}"]
    if isinstance(data, list):
        data = data[0] if data else {}
    if not isinstance(data, dict):
        return ["JSON is not an object"]
    failures = [f"JSON missing key {k}" for k in expect["json_keys"] if k not in data]
    for key, value in expect.get("json_values", {}).items():
        got = data.get(key)
        if value is None and got not in (None, "", "null"):
            failures.append(f"JSON {key}={got!r}, expected null")
        elif value is not None and norm(value) not in norm(got or ""):
            failures.append(f"JSON {key}={got!r}, expected {value!r}")
    return failures


def run_case(client, case, skills, tools, max_tokens):
    index = [skills[n] for n in case["index"]]
    routed = [skills[n] for n in case["routed"]]
    offered = [tools[n] for n in case["tools"]]
    messages = [
        {"role": "system", "content": build_system_prompt(index, routed)},
        {"role": "user", "content": case["user"]},
    ]
    reply = client.chat(messages, tools=offered or None, temperature=0.0, max_tokens=max_tokens)
    failures = check_reply(reply, case["expect"], offered)
    transcript = [reply]
    if not failures and "tool_result" in case and reply["tool_calls"]:
        call = reply["tool_calls"][0]
        args, _ = check_call(call, offered)
        messages.append(assistant_tool_message([{"name": call["name"], "arguments": args}]))
        messages.append(
            {"role": "tool", "tool_call_id": "call_0", "name": call["name"], "content": json.dumps(case["tool_result"])}
        )
        final = client.chat(messages, tools=offered, temperature=0.0, max_tokens=max_tokens)
        transcript.append(final)
        if final["tool_calls"]:
            failures.append("called another tool instead of answering from the result")
        else:
            failures += [f"final answer: {f}" for f in check_text(final["content"], case.get("final", {}))]
    return {"id": case["id"], "category": case["category"], "passed": not failures, "failures": failures, "replies": transcript}


def summarize(results, label):
    by_category = defaultdict(list)
    for r in results:
        by_category[r["category"]].append(r["passed"])
    calls = [c for r in results for reply in r["replies"] for c in reply["tool_calls"]]
    malformed = sum(1 for r in results if any("malformed" in f or "invalid call" in f for f in r["failures"]))
    summary = {
        "label": label,
        "pass_rate": sum(r["passed"] for r in results) / max(1, len(results)),
        "by_category": {k: sum(v) / len(v) for k, v in sorted(by_category.items())},
        "tool_calls_made": len(calls),
        "cases_with_broken_tool_calls": malformed,
    }
    return summary


def print_summary(summary):
    print(f"\n{summary['label']}: {summary['pass_rate']:.0%} of cases passed")
    for category, rate in summary["by_category"].items():
        print(f"  {category:<20} {rate:.0%}")
    print(f"  broken tool calls in {summary['cases_with_broken_tool_calls']} case(s)")


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--base-url", default="http://127.0.0.1:8081/v1")
    p.add_argument("--model", default="model")
    p.add_argument("--label", default="model")
    p.add_argument("--out", help="write the full report (every reply and failure) here")
    p.add_argument("--max-tokens", type=int, default=768)
    p.add_argument(
        "--extra-body",
        default="{}",
        help='JSON merged into every request, e.g. \'{"chat_template_kwargs": {"enable_thinking": false}}\'',
    )
    args = p.parse_args(argv)

    skills, tools = load_fixtures()
    client = ChatClient(args.base_url, args.model, extra_body=json.loads(args.extra_body))
    cases = [json.loads(line) for line in (EVAL_DIR / "cases.jsonl").read_text(encoding="utf-8").splitlines() if line.strip()]
    results = []
    for case in cases:
        result = run_case(client, case, skills, tools, args.max_tokens)
        mark = "PASS" if result["passed"] else "FAIL"
        print(f"{mark} {case['id']}" + ("" if result["passed"] else f": {'; '.join(result['failures'])}"), flush=True)
        results.append(result)
    summary = summarize(results, args.label)
    print_summary(summary)
    if args.out:
        Path(args.out).write_text(json.dumps({"summary": summary, "results": results}, indent=1, ensure_ascii=False))
    return summary


if __name__ == "__main__":
    main()
