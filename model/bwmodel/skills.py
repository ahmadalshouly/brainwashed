"""Reads SKILL.md files the same way crates/skills does."""

import re
from dataclasses import dataclass, field
from pathlib import Path

import yaml

NAME_RE = re.compile(r"^[a-z0-9-]+$")


@dataclass(frozen=True)
class Skill:
    name: str
    description: str
    body: str
    triggers: tuple = field(default_factory=tuple)

    def to_markdown(self):
        triggers = ", ".join(self.triggers)
        return (
            f"---\nname: {self.name}\ndescription: {self.description}\n"
            f"triggers: [{triggers}]\nversion: 1\n---\n\n{self.body}\n"
        )


def parse_skill(source):
    source = source.lstrip("﻿").replace("\r\n", "\n")
    if not source.startswith("---\n"):
        raise ValueError("skill file must start with a `---` frontmatter block")
    rest = source[4:]
    end = rest.find("\n---")
    if end < 0:
        raise ValueError("skill file must start with a `---` frontmatter block")
    meta = yaml.safe_load(rest[:end]) or {}
    name = str(meta.get("name", ""))
    if not NAME_RE.match(name):
        raise ValueError(f"skill `name` must be lowercase letters, digits and dashes, got `{name}`")
    description = str(meta.get("description", "")).strip()
    if not description:
        raise ValueError(f"skill `{name}` has no description")
    body = rest[end + 4 :].lstrip("\r\n").rstrip()
    triggers = tuple(str(t) for t in meta.get("triggers") or [])
    return Skill(name=name, description=description, body=body, triggers=triggers)


def load_skills(*folders):
    """Loads every `<folder>/<name>/SKILL.md`, sorted by name."""
    skills = {}
    for folder in folders:
        for path in sorted(Path(folder).glob("*/SKILL.md")):
            skill = parse_skill(path.read_text(encoding="utf-8"))
            skills[skill.name] = skill
    return [skills[name] for name in sorted(skills)]
