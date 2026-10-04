import json
import unittest
from pathlib import Path

from bwmodel.prompt import DEFAULT_SYSTEM_PROMPT, build_system_prompt
from bwmodel.skills import load_skills, parse_skill
from bwmodel.tools import check_call, load_tools, validate

MODEL_DIR = Path(__file__).resolve().parents[1]


class PromptTest(unittest.TestCase):
    def test_matches_host_format(self):
        a = parse_skill("---\nname: alpha\ndescription: Does A.\n---\n\nStep one.\n")
        b = parse_skill("---\nname: beta\ndescription: Does B.\n---\nDo B.")
        prompt = build_system_prompt([a, b], [b])
        self.assertEqual(
            prompt,
            DEFAULT_SYSTEM_PROMPT
            + "\n\n# Skills\nYou have these skills. When one fits the request, its instructions appear below and you must follow them.\n"
            "- alpha: Does A.\n- beta: Does B.\n"
            "\n## Skill: beta\nDo B.\n",
        )

    def test_no_skills_means_plain_prompt(self):
        self.assertEqual(build_system_prompt([], []), DEFAULT_SYSTEM_PROMPT)
        self.assertTrue(DEFAULT_SYSTEM_PROMPT.endswith("own computer. Answer clearly and concisely."))


class FixturesTest(unittest.TestCase):
    def test_all_skills_parse(self):
        skills = load_skills(
            MODEL_DIR / "data/seeds/skills/train", MODEL_DIR / "eval/skills", MODEL_DIR.parent / "skills-examples"
        )
        self.assertGreaterEqual(len(skills), 18)

    def test_train_and_eval_do_not_overlap(self):
        train = {s.name for s in load_skills(MODEL_DIR / "data/seeds/skills/train")}
        held_out = {s.name for s in load_skills(MODEL_DIR / "eval/skills", MODEL_DIR.parent / "skills-examples")}
        self.assertFalse(train & held_out)
        train_tools = {t["function"]["name"] for t in load_tools(MODEL_DIR / "data/seeds/tools.json")}
        eval_tools = {t["function"]["name"] for t in load_tools(MODEL_DIR / "eval/tools.json")}
        self.assertFalse(train_tools & eval_tools)

    def test_eval_cases_reference_real_fixtures(self):
        skills = {s.name for s in load_skills(MODEL_DIR / "eval/skills", MODEL_DIR.parent / "skills-examples")}
        tools = {t["function"]["name"] for t in load_tools(MODEL_DIR / "eval/tools.json")}
        for line in (MODEL_DIR / "eval/cases.jsonl").read_text().splitlines():
            case = json.loads(line)
            self.assertLessEqual(set(case["index"]) | set(case["routed"]), skills, case["id"])
            self.assertLessEqual(set(case["routed"]), set(case["index"]), case["id"])
            self.assertLessEqual(set(case["tools"]), tools, case["id"])
            wanted = case["expect"].get("tool")
            if wanted:
                self.assertIn(wanted, case["tools"], case["id"])


class ToolsTest(unittest.TestCase):
    schema = {
        "type": "object",
        "properties": {"city": {"type": "string"}, "unit": {"type": "string", "enum": ["c", "f"]}, "n": {"type": "integer"}},
        "required": ["city"],
    }

    def test_validate(self):
        self.assertEqual(validate({"city": "Oslo", "unit": "c", "n": 2}, self.schema), [])
        self.assertTrue(validate({}, self.schema))
        self.assertTrue(validate({"city": "Oslo", "unit": "kelvin"}, self.schema))
        self.assertTrue(validate({"city": "Oslo", "n": True}, self.schema))
        self.assertTrue(validate({"city": "Oslo", "made_up": 1}, self.schema))

    def test_check_call_parses_string_arguments(self):
        tools = [{"type": "function", "function": {"name": "w", "description": "d", "parameters": self.schema}}]
        args, problems = check_call({"name": "w", "arguments": '{"city": "Oslo"}'}, tools)
        self.assertEqual((args, problems), ({"city": "Oslo"}, []))
        self.assertTrue(check_call({"name": "w", "arguments": "{oops"}, tools)[1])
        self.assertTrue(check_call({"name": "nope", "arguments": "{}"}, tools)[1])


if __name__ == "__main__":
    unittest.main()
