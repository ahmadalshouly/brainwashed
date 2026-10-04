"""Runs the data generator and the eval end to end against a fake teacher."""

import io
import itertools
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

from data import generate
from eval import run_eval
from tests.fake_openai import FakeServer, text, tool_call

counter = itertools.count()


def fake_arguments(params):
    values = {"string": "x", "integer": 1, "number": 1.5, "boolean": True, "array": ["a"], "object": {}}
    out = {}
    for key in params.get("required", []):
        prop = params["properties"][key]
        out[key] = prop["enum"][0] if "enum" in prop else values[prop.get("type", "string")]
    return out


def fake_teacher(body):
    messages = body["messages"]
    system = messages[0]["content"] if messages[0]["role"] == "system" else ""
    last = messages[-1]["content"]
    if system == generate.SCENARIO_SYSTEM:
        return text('{"user": "please help me with this"}')
    if system.startswith("You simulate software tools"):
        return text('{"ok": true, "value": 42}')
    if last.startswith("Invent one skill"):
        n = next(counter)
        return text(json.dumps({"name": f"gen-skill-{n}", "description": "Does a thing.", "triggers": ["thing"], "body": "1. Do it."}))
    if last.startswith("Invent 5 realistic tools"):
        n = next(counter)
        tool = {"type": "function", "function": {"name": f"gen_tool_{n}", "description": "d", "parameters": {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]}}}
        return text(json.dumps({"tools": [tool]}))
    # The responder. The hidden note says what kind of reply is wanted.
    no_tool = "does not need any tool" in system or "left out a required detail" in system
    if body.get("tools") and not no_tool and messages[-1]["role"] == "user":
        fn = body["tools"][0]["function"]
        return tool_call(fn["name"], fake_arguments(fn["parameters"]))
    return text("Here is the answer. Anything else?")


class GenerateTest(unittest.TestCase):
    def test_end_to_end(self):
        with tempfile.TemporaryDirectory() as out, FakeServer(fake_teacher) as server, redirect_stdout(io.StringIO()):
            argv = ["--base-url", server.url, "--out", out, "--num", "60", "--gen-skills", "8", "--gen-tools", "5", "--workers", "4"]
            generate.main(argv)
            rows = [json.loads(l) for l in (Path(out) / "train.jsonl").read_text().splitlines()]
            rows += [json.loads(l) for l in (Path(out) / "val.jsonl").read_text().splitlines()]
            rejected = (Path(out) / "rejected.jsonl").read_text().splitlines()
            self.assertEqual(len(rows) + len(rejected), 60)
            self.assertGreater(len(rows), 45)
            self.assertEqual(len(list((Path(out) / "skills").glob("*/SKILL.md"))), 8)

            reserved_skills, reserved_tools = generate.reserved_names()
            categories = set()
            for row in rows:
                categories.add(row["category"])
                system = row["messages"][0]
                self.assertEqual(system["role"], "system")
                self.assertNotIn("Hidden note", system["content"])
                self.assertFalse(set(row["skills_index"]) & reserved_skills)
                self.assertFalse({t["function"]["name"] for t in row.get("tools", [])} & reserved_tools)
                for name in row["skills_routed"]:
                    self.assertIn(f"## Skill: {name}\n", system["content"])
                self.assertEqual(row["messages"][-1]["role"], "assistant")
                for m in row["messages"]:
                    for call in m.get("tool_calls", []):
                        self.assertIsInstance(call["function"]["arguments"], dict)
                if row["category"] == "tool_call":
                    self.assertIn("tool_calls", row["messages"][2])
                    self.assertEqual(row["messages"][3]["role"], "tool")
            self.assertGreaterEqual(len(categories), 6)

            # A rerun resumes instead of starting over.
            before = len(server.requests)
            generate.main(argv)
            self.assertEqual(len(server.requests), before)




class EvalTest(unittest.TestCase):
    def test_checks(self):
        offered = [{"type": "function", "function": {"name": "get_air_quality", "description": "d", "parameters": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}}}]
        call = {"content": "", "tool_calls": [{"name": "get_air_quality", "arguments": '{"city": "New Delhi"}'}]}
        self.assertEqual(run_eval.check_reply(call, {"tool": "get_air_quality", "args": {"city": "delhi"}}, offered), [])
        self.assertTrue(run_eval.check_reply(call, {"tool": None}, offered))
        raw = {"content": '<tool_call>{"name": "get_air_quality"', "tool_calls": []}
        self.assertIn("malformed", run_eval.check_reply(raw, {"tool": "get_air_quality"}, offered)[0])
        reply = {"content": "Yesterday: a\nToday: b\nBlockers: none", "tool_calls": []}
        self.assertEqual(run_eval.check_reply(reply, {"tool": None, "include": [r"^[\s*_-]*Blockers:\W*none"], "max_words": 20}, []), [])
        js = {"content": '```json\n{"name": "Tom", "email": null, "phone": "1", "company": null}\n```', "tool_calls": []}
        self.assertEqual(run_eval.check_text(js["content"], {"json_keys": ["name", "email"], "json_values": {"name": "tom", "email": None}}), [])

    def test_runs_against_a_server(self):
        def model(body):
            if body.get("tools") and body["messages"][-1]["role"] == "user":
                fn = body["tools"][0]["function"]
                return tool_call(fn["name"], fake_arguments(fn["parameters"]))
            return text("Yesterday: x\nToday: y\nBlockers: none")

        with tempfile.TemporaryDirectory() as tmp, FakeServer(model) as server, redirect_stdout(io.StringIO()):
            summary = run_eval.main(["--base-url", server.url, "--out", f"{tmp}/r.json"])
            report = json.loads(Path(f"{tmp}/r.json").read_text())
        self.assertEqual(len(report["results"]), len((run_eval.EVAL_DIR / "cases.jsonl").read_text().splitlines()))
        self.assertTrue(0 < summary["pass_rate"] < 1)


if __name__ == "__main__":
    unittest.main()
