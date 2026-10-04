"""Generates BrainWashed fine-tuning data with a larger "teacher" model.

Every example is built with the host's real prompt format (skill index plus the
1-2 routed skill bodies, and tools in the OpenAI format), so the small model
learns exactly what it will see in the app. The teacher gets one extra hidden
note explaining what a perfect reply looks like; that note is never saved.

The teacher is any OpenAI-compatible endpoint: llama-server on Kaggle (see
notebooks/01_generate_data.ipynb) or a hosted API. Use an open-weight teacher
whose license lets you train on its outputs (e.g. Qwen3, Apache-2.0).

    python model/data/generate.py --base-url http://127.0.0.1:8080/v1 \
        --model teacher --out /kaggle/working/data --num 3000 --gen-skills 150

Runs are resumable: rerun the same command and finished samples are skipped.
"""

import argparse
import json
import os
import random
import re
import sys
import threading
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

MODEL_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(MODEL_DIR))

from bwmodel.client import ChatClient, assistant_tool_message, parse_json_reply  # noqa: E402
from bwmodel.prompt import DEFAULT_SYSTEM_PROMPT, build_system_prompt  # noqa: E402
from bwmodel.skills import Skill, load_skills, parse_skill  # noqa: E402
from bwmodel.tools import check_call, check_tool_definition, load_tools, tool_name  # noqa: E402

SEEDS = MODEL_DIR / "data" / "seeds"
EVAL_DIR = MODEL_DIR / "eval"
REPO_SKILLS = MODEL_DIR.parent / "skills-examples"

# Share of each kind of example. See README for what each one teaches.
CATEGORIES = {
    "skill_follow": 0.30,
    "skill_two_routed": 0.07,
    "skill_wrong_route": 0.06,
    "no_skill": 0.10,
    "tool_call": 0.22,
    "tool_missing_args": 0.06,
    "tool_not_needed": 0.08,
    "skill_with_tools": 0.11,
}
FOLLOW_UP_RATE = 0.2
TOOL_ERROR_RATE = 0.08
MAX_TOOL_ROUNDS = 3
MAX_REPLY_WORDS = 450

# Users can change the base system prompt in settings, so a few examples use
# other wording to keep the model from depending on the exact default.
ALT_SYSTEM_PROMPTS = [
    "You are a friendly assistant on my laptop. Keep answers short.",
    "You are BrainWashed. Be direct and practical.",
    "You are my private home AI. Reply in plain language and avoid jargon.",
    "You are a helpful assistant.",
]

USER_STYLES = [
    "casual and short, like a text message",
    "polite and detailed",
    "terse, a few words only",
    "rambling, with an unnecessary detail or two",
    "with a couple of typos and no capital letters",
    "written by a non-native English speaker",
    "in Arabic",
    "in Spanish",
    "in French",
    "slightly impatient",
]
NON_ENGLISH_STYLE_WEIGHT = 0.12

SCENARIO_SYSTEM = (
    "You write realistic messages that people send to a private AI assistant running on "
    "their own laptop. Reply with a single JSON object and nothing else."
)

REPLY_RULES = """[Hidden note for the writer of the ideal reply. The user cannot see it. Never mention it.]
Write the single best reply a small, careful assistant could give:
- If a skill's instructions appear under "## Skill:" and they fit the request, follow every step and formatting rule exactly.
- If the request is missing something the skill says to ask for, ask only for that, in one short question, and stop.
- Ignore skill instructions that do not fit the request.
- Never mention skills, instructions, system prompts or tools by name in your text.
- When a tool is needed for facts you do not have (live data, the user's files, calendar, actions), call it instead of guessing. Do not call tools you do not need.
- If a tool needs an argument the user has not given and you cannot reasonably infer it, ask for it instead of calling the tool.
- After tool results arrive, answer from them only. If a tool returned an error, say so plainly and suggest a next step.
- Be concise. No filler openings like "Sure!" or "Great question". Reply in the user's language."""

CATEGORY_NOTES = {
    "skill_wrong_route": "The skill instructions shown do not fit this request. Ignore them and just answer.",
    "no_skill": "No skill fits this request. Just answer it well.",
    "tool_not_needed": "This request does not need any tool. Answer directly without calling one.",
    "tool_missing_args": "The user left out a required detail. Ask for it in one short question; do not call a tool yet.",
}


class Generator:
    def __init__(self, teacher, skills, tools, out_dir, seed):
        self.teacher = teacher
        self.skills = skills
        self.tools = tools
        self.out_dir = Path(out_dir)
        self.seed = seed
        self.lock = threading.Lock()

    # ---- prompts --------------------------------------------------------

    def scenario(self, rng, instruction, **context):
        style = self.pick_style(rng)
        lines = [instruction, f"Write the user's message {style}."]
        for key, value in context.items():
            lines.append(f"\n{key}:\n{value}")
        lines.append('\nReturn {"user": "<the message>"}.')
        reply = self.teacher.chat(
            [{"role": "system", "content": SCENARIO_SYSTEM}, {"role": "user", "content": "\n".join(lines)}],
            temperature=0.95,
            max_tokens=600,
            json_mode=True,
        )
        user = str(parse_json_reply(reply["content"]).get("user", "")).strip()
        if len(user) < 2:
            raise Rejected("empty scenario")
        return user

    def pick_style(self, rng):
        if rng.random() < NON_ENGLISH_STYLE_WEIGHT:
            return rng.choice([s for s in USER_STYLES if s.startswith("in ")])
        return rng.choice([s for s in USER_STYLES if not s.startswith("in ")])

    def system_prompt(self, rng, index, routed):
        base = DEFAULT_SYSTEM_PROMPT if rng.random() < 0.85 else rng.choice(ALT_SYSTEM_PROMPTS)
        return build_system_prompt(sorted(index, key=lambda s: s.name), routed, base=base)

    def pick_index(self, rng, must_include, low=2, high=8):
        others = [s for s in self.skills if s not in must_include]
        count = max(0, rng.randint(low, high) - len(must_include))
        return list(must_include) + rng.sample(others, min(count, len(others)))

    def pick_tools(self, rng, must_include=(), low=2, high=6):
        others = [t for t in self.tools if t not in must_include]
        count = max(0, rng.randint(low, high) - len(must_include))
        chosen = list(must_include) + rng.sample(others, min(count, len(others)))
        rng.shuffle(chosen)
        return chosen

    # ---- the conversation loop ------------------------------------------

    def respond(self, rng, messages, tools, note, expect):
        """Asks the teacher for the next assistant turn(s), running simulated
        tools until it answers in text. Appends to `messages`."""
        hidden = REPLY_RULES + (f"\n- {note}" if note else "")
        for _ in range(MAX_TOOL_ROUNDS + 1):
            teacher_view = [{**messages[0], "content": messages[0]["content"] + "\n\n" + hidden}] + messages[1:]
            reply = self.teacher.chat(teacher_view, tools=tools or None, temperature=0.5, max_tokens=1200)
            if reply["tool_calls"]:
                if expect in ("text", "question"):
                    raise Rejected("called a tool when none was wanted")
                calls = []
                for call in reply["tool_calls"][:3]:
                    args, problems = check_call(call, tools or [])
                    if problems:
                        raise Rejected(f"invalid tool call: {problems}")
                    calls.append({"name": call["name"], "arguments": args})
                messages.append(assistant_tool_message(calls))
                for i, call in enumerate(calls):
                    messages.append(
                        {
                            "role": "tool",
                            "tool_call_id": f"call_{i}",
                            "name": call["name"],
                            "content": self.simulate_tool(rng, call, tools),
                        }
                    )
                expect = "any"
                continue
            if expect == "tool":
                raise Rejected("answered in text when a tool call was wanted")
            text = clean_text(reply["content"])
            if expect == "question" and "?" not in text and "؟" not in text:
                raise Rejected("expected a clarifying question")
            messages.append({"role": "assistant", "content": text})
            return
        raise Rejected("too many tool rounds")

    def simulate_tool(self, rng, call, tools):
        tool = next(t for t in tools if tool_name(t) == call["name"])["function"]
        failing = rng.random() < TOOL_ERROR_RATE
        prompt = (
            f"You are simulating the tool `{tool['name']}`: {tool['description']}\n"
            f"It was called with these arguments:\n{json.dumps(call['arguments'], ensure_ascii=False)}\n\n"
            + (
                'This call fails. Return {"error": "<a realistic short error message>"}.'
                if failing
                else "Return the JSON object the real tool would return: realistic, concrete, compact "
                "(under 120 words of data). Return only the JSON object."
            )
        )
        reply = self.teacher.chat(
            [{"role": "system", "content": "You simulate software tools. Reply with JSON only."}, {"role": "user", "content": prompt}],
            temperature=0.7,
            max_tokens=500,
            json_mode=True,
        )
        return json.dumps(parse_json_reply(reply["content"]), ensure_ascii=False)

    def follow_up(self, rng, messages, tools, note):
        transcript = "\n".join(
            f"{m['role']}: {m['content']}" for m in messages[1:] if m["role"] in ("user", "assistant") and m.get("content")
        )
        user = self.scenario(
            rng,
            "Write the user's natural next message in this conversation: a follow-up, a correction, "
            "an answer to the assistant's question, or a related request.",
            Conversation=transcript[-3000:],
        )
        messages.append({"role": "user", "content": user})
        self.respond(rng, messages, tools, note, expect="any")

    # ---- categories -----------------------------------------------------

    def build(self, sample_id):
        rng = random.Random(f"{self.seed}:{sample_id}")
        category = rng.choices(list(CATEGORIES), weights=list(CATEGORIES.values()))[0]
        row = getattr(self, f"build_{category}")(rng)
        row["category"] = category
        if category not in ("skill_wrong_route", "tool_missing_args") and rng.random() < FOLLOW_UP_RATE:
            self.follow_up(rng, row["messages"], row.get("tools"), CATEGORY_NOTES.get(category))
        row["id"] = sample_id
        return row

    def skill_row(self, rng, index, routed, user, tools=None, note=None, expect="text"):
        messages = [
            {"role": "system", "content": self.system_prompt(rng, index, routed)},
            {"role": "user", "content": user},
        ]
        self.respond(rng, messages, tools, note, expect)
        row = {
            "skills_index": sorted(s.name for s in index),
            "skills_routed": [s.name for s in routed],
            "messages": messages,
        }
        if tools:
            row["tools"] = tools
        return row

    def build_skill_follow(self, rng):
        skill = rng.choice(self.skills)
        underspecified = rng.random() < 0.3
        instruction = (
            "Write a request this skill is meant for, but leave out a detail the skill says to ask "
            "about. If the skill never asks for anything, write a complete request instead."
            if underspecified
            else "Write a request this skill is meant for, with enough detail to answer it."
        )
        user = self.scenario(rng, instruction, Skill=skill.to_markdown())
        return self.skill_row(rng, self.pick_index(rng, [skill]), [skill], user)

    def build_skill_two_routed(self, rng):
        target, other = rng.sample(self.skills, 2)
        user = self.scenario(rng, "Write a request this skill is meant for.", Skill=target.to_markdown())
        routed = [target, other] if rng.random() < 0.7 else [other, target]
        note = f"Only the `{target.name}` instructions fit this request; ignore the others."
        return self.skill_row(rng, self.pick_index(rng, [target, other]), routed, user, note=note)

    def build_skill_wrong_route(self, rng):
        skill = rng.choice(self.skills)
        user = self.scenario(
            rng,
            "Write a message that shares a word or two with this skill's topic but actually asks for "
            "something different that the skill does not cover.",
            Skill=skill.to_markdown(),
        )
        return self.skill_row(rng, self.pick_index(rng, [skill]), [skill], user, note=CATEGORY_NOTES["skill_wrong_route"])

    def build_no_skill(self, rng):
        index = self.pick_index(rng, [], low=0, high=6)
        user = self.scenario(
            rng,
            "Write an everyday question or request (facts, advice, writing, coding, math, chit-chat) "
            "that none of these skills cover.",
            Skills="\n".join(f"- {s.name}: {s.description}" for s in index) or "(none)",
        )
        return self.skill_row(rng, index, [], user, note=CATEGORY_NOTES["no_skill"])

    def tool_scenario(self, rng, tool, missing):
        instruction = (
            "Write a request that needs this tool, but leave out one required argument that cannot be "
            "guessed (for example which city, which file, what time or which recipient)."
            if missing
            else "Write a request that can only be answered well by calling this tool. Include the "
            "details the tool's required arguments need."
        )
        return self.scenario(rng, instruction, Tool=json.dumps(tool["function"], ensure_ascii=False))

    def build_tool_call(self, rng):
        tool = rng.choice(self.tools)
        user = self.tool_scenario(rng, tool, missing=False)
        index = self.pick_index(rng, [], low=0, high=5)
        return self.skill_row(rng, index, [], user, tools=self.pick_tools(rng, [tool]), expect="tool")

    def build_tool_missing_args(self, rng):
        tool = rng.choice([t for t in self.tools if t["function"]["parameters"].get("required")])
        user = self.tool_scenario(rng, tool, missing=True)
        index = self.pick_index(rng, [], low=0, high=5)
        return self.skill_row(
            rng, index, [], user, tools=self.pick_tools(rng, [tool]), note=CATEGORY_NOTES["tool_missing_args"], expect="question"
        )

    def build_tool_not_needed(self, rng):
        tools = self.pick_tools(rng)
        user = self.scenario(
            rng,
            "Write a request a good assistant can answer from general knowledge or writing skill alone, "
            "so none of these tools is needed (for example explaining a concept, rewriting text or giving advice).",
            Tools="\n".join(f"- {tool_name(t)}: {t['function']['description']}" for t in tools),
        )
        index = self.pick_index(rng, [], low=0, high=5)
        return self.skill_row(rng, index, [], user, tools=tools, note=CATEGORY_NOTES["tool_not_needed"])

    def build_skill_with_tools(self, rng):
        skill = rng.choice(self.skills)
        tools = self.pick_tools(rng, low=3, high=6)
        user = self.scenario(
            rng,
            "Write a request this skill is meant for, where following the skill would naturally use one "
            "of these tools (for example looking something up, saving the result or checking the date). "
            "If no tool fits naturally, just write a normal request for the skill.",
            Skill=skill.to_markdown(),
            Tools="\n".join(f"- {tool_name(t)}: {t['function']['description']}" for t in tools),
        )
        return self.skill_row(rng, self.pick_index(rng, [skill]), [skill], user, tools=tools, expect="any")

    # ---- driver ---------------------------------------------------------

    def run(self, num, workers):
        samples = self.out_dir / "samples.jsonl"
        rejected = self.out_dir / "rejected.jsonl"
        done = read_ids(samples) | read_ids(rejected)
        todo = [f"s{i:06d}" for i in range(num) if f"s{i:06d}" not in done]
        print(f"{len(done)} already done, generating {len(todo)}", flush=True)
        kept = failed = errors_in_a_row = 0
        with ThreadPoolExecutor(workers) as pool, samples.open("a") as ok, rejected.open("a") as bad:
            futures = {pool.submit(self.build, sid): sid for sid in todo}
            for n, future in enumerate(as_completed(futures), 1):
                sid = futures[future]
                try:
                    line, out = future.result(), ok
                    kept += 1
                except (Rejected, ValueError, KeyError, TypeError, json.JSONDecodeError) as e:
                    line, out = {"id": sid, "reason": str(e)[:300]}, bad
                    failed += 1
                except (RuntimeError, OSError) as e:
                    # Teacher unreachable: not recorded, so a rerun retries it.
                    errors_in_a_row += 1
                    print(f"{sid}: teacher error: {e}", flush=True)
                    if errors_in_a_row >= 20:
                        pool.shutdown(cancel_futures=True)
                        raise SystemExit("teacher keeps failing; fix it and rerun to resume") from e
                    continue
                errors_in_a_row = 0
                with self.lock:
                    out.write(json.dumps(line, ensure_ascii=False) + "\n")
                    out.flush()
                if n % 25 == 0 or n == len(todo):
                    print(f"{n}/{len(todo)}  kept {kept}  rejected {failed}", flush=True)


class Rejected(Exception):
    pass


def clean_text(text):
    text = re.sub(r"<think>.*?</think>", "", text, flags=re.S).strip()
    lowered = text.lower()
    if not text:
        raise Rejected("empty reply")
    if "<think>" in lowered or "hidden note" in lowered or "system prompt" in lowered:
        raise Rejected("reply leaks the hidden note or thinking")
    if len(text.split()) > MAX_REPLY_WORDS:
        raise Rejected("reply too long")
    return text


def read_ids(path):
    if not path.exists():
        return set()
    ids = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            ids.add(json.loads(line)["id"])
        except (json.JSONDecodeError, KeyError):
            pass  # a run killed mid-write leaves a partial last line
    return ids


# ---- teacher-written skills and tools -------------------------------------


def generate_skills(teacher, out_dir, count, reserved, seed, workers):
    """Asks the teacher to invent `count` new skills, so the student learns to
    follow skills in many styles rather than memorizing a handful."""
    folder = Path(out_dir) / "skills"
    folder.mkdir(parents=True, exist_ok=True)
    have = len(list(folder.glob("*/SKILL.md")))
    if have >= count:
        return
    topics = [t.strip() for t in (SEEDS / "skill_topics.txt").read_text().splitlines() if t.strip() and not t.startswith("#")]
    examples = load_skills(SEEDS / "skills" / "train")
    print(f"generating {count - have} skills", flush=True)

    def make(i):
        rng = random.Random(f"{seed}:skill:{i}")
        shots = rng.sample(examples, 2)
        prompt = (
            "Invent one skill file for a small local AI assistant. A skill is a short set of instructions "
            "the assistant must follow when a request matches it.\n\n"
            f"Topic: {rng.choice(topics)}\n"
            f"Make it {rng.choice(['3', '4', '5', '6'])} instructions long. "
            + rng.choice(
                [
                    "Include one step where the assistant asks for missing information first.",
                    "Include a strict output format (headings, a table or a fixed template).",
                    "Include a word limit and one thing the assistant must never do.",
                    "Use bullet points instead of numbered steps.",
                ]
            )
            + "\n\nTwo examples of the format:\n\n"
            + "\n\n".join(s.to_markdown() for s in shots)
            + '\nReturn JSON: {"name": "lowercase-with-dashes", "description": "one sentence", '
            '"triggers": ["3-4 short phrases users might type"], "body": "the markdown instructions"}'
        )
        reply = teacher.chat([{"role": "user", "content": prompt}], temperature=1.0, max_tokens=900, json_mode=True)
        data = parse_json_reply(reply["content"])
        skill = Skill(
            name=str(data["name"]).strip().lower(),
            description=str(data["description"]).strip(),
            body=str(data["body"]).strip(),
            triggers=tuple(str(t) for t in data.get("triggers") or []),
        )
        skill = parse_skill(skill.to_markdown())  # same validation the host does
        return skill

    with ThreadPoolExecutor(workers) as pool:
        futures = [pool.submit(make, i) for i in range(have, count)]
        for future in as_completed(futures):
            try:
                skill = future.result()
            except (ValueError, KeyError, RuntimeError, json.JSONDecodeError) as e:
                print(f"skipped a skill: {e}", flush=True)
                continue
            if skill.name in reserved or (folder / skill.name).exists():
                continue
            (folder / skill.name).mkdir()
            (folder / skill.name / "SKILL.md").write_text(skill.to_markdown(), encoding="utf-8")


def generate_tools(teacher, out_dir, count, reserved, seed):
    """Asks the teacher for extra tool definitions so the student learns to
    read any schema, not just the seed tools."""
    path = Path(out_dir) / "tools.json"
    tools = json.loads(path.read_text()) if path.exists() else []
    names = {tool_name(t) for t in tools} | set(reserved)
    i = 0
    while len(tools) < count and i < count:
        rng = random.Random(f"{seed}:tools:{i}")
        i += 1
        prompt = (
            "Invent 5 realistic tools (functions) a personal AI assistant on a laptop could call, "
            f"in the area of: {rng.choice(['home and family', 'work and productivity', 'health and fitness', 'money', 'travel', 'media and hobbies', 'the computer itself', 'learning', 'shopping', 'communication'])}. "
            "Vary the parameter types (string, integer, number, boolean, enum, array). "
            'Return JSON: {"tools": [{"type": "function", "function": {"name": "snake_case", '
            '"description": "...", "parameters": {"type": "object", "properties": {...}, "required": [...]}}}]}'
        )
        try:
            reply = teacher.chat([{"role": "user", "content": prompt}], temperature=1.0, max_tokens=2000, json_mode=True)
            batch = parse_json_reply(reply["content"]).get("tools", [])
        except (ValueError, RuntimeError, json.JSONDecodeError) as e:
            print(f"skipped a tool batch: {e}", flush=True)
            continue
        for tool in batch:
            try:
                check_tool_definition(tool)
            except (ValueError, AttributeError, TypeError):
                continue
            name = tool_name(tool)
            if re.fullmatch(r"[a-z][a-z0-9_]*", name) and name not in names:
                names.add(name)
                tools.append(tool)
    path.write_text(json.dumps(tools[:count], indent=1, ensure_ascii=False))


# ---- splitting --------------------------------------------------------------


def split(out_dir, val_fraction, seed):
    rows = [json.loads(line) for line in (Path(out_dir) / "samples.jsonl").read_text(encoding="utf-8").splitlines() if line.strip()]
    random.Random(seed).shuffle(rows)
    n_val = max(1, int(len(rows) * val_fraction)) if len(rows) > 1 else 0
    for name, part in (("val.jsonl", rows[:n_val]), ("train.jsonl", rows[n_val:])):
        with (Path(out_dir) / name).open("w") as f:
            for row in part:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")
    counts = {}
    for row in rows:
        counts[row["category"]] = counts.get(row["category"], 0) + 1
    print(f"train {len(rows) - n_val}, val {n_val}; by category: {json.dumps(counts)}")


def reserved_names():
    """Skill and tool names kept out of training so the eval stays honest."""
    skills = {s.name for s in load_skills(EVAL_DIR / "skills", REPO_SKILLS)}
    tools = {tool_name(t) for t in load_tools(EVAL_DIR / "tools.json")}
    return skills, tools


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--base-url", default=os.environ.get("TEACHER_BASE_URL", "http://127.0.0.1:8080/v1"))
    p.add_argument("--model", default=os.environ.get("TEACHER_MODEL", "teacher"))
    p.add_argument("--api-key-env", default="TEACHER_API_KEY", help="environment variable holding the API key, if any")
    p.add_argument("--out", required=True)
    p.add_argument("--num", type=int, default=3000, help="number of conversations to attempt")
    p.add_argument("--gen-skills", type=int, default=150, help="teacher-written skills to add to the seeds")
    p.add_argument("--gen-tools", type=int, default=60, help="teacher-written tools to add to the seeds")
    p.add_argument("--workers", type=int, default=8)
    p.add_argument("--val-fraction", type=float, default=0.05)
    p.add_argument("--seed", type=int, default=1)
    args = p.parse_args(argv)

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    teacher = ChatClient(args.base_url, args.model, api_key=os.environ.get(args.api_key_env))
    reserved_skills, reserved_tools = reserved_names()

    if args.gen_skills:
        generate_skills(teacher, out, args.gen_skills, reserved_skills, args.seed, args.workers)
    if args.gen_tools:
        generate_tools(teacher, out, args.gen_tools, reserved_tools, args.seed)

    skills = [s for s in load_skills(SEEDS / "skills" / "train", out / "skills") if s.name not in reserved_skills]
    tools = load_tools(SEEDS / "tools.json")
    if (out / "tools.json").exists():
        tools += load_tools(out / "tools.json")
    tools = [t for t in tools if tool_name(t) not in reserved_tools]
    print(f"{len(skills)} skills, {len(tools)} tools", flush=True)

    Generator(teacher, skills, tools, out, args.seed).run(args.num, args.workers)
    split(out, args.val_fraction, args.seed)


if __name__ == "__main__":
    main()
