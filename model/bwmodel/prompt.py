"""Builds the system prompt the host sends to the model.

Mirrors `Engine::build_prompt` in crates/core/src/skills.rs and the default in
crates/core/src/settings.rs. If the host's prompt changes, change it here too
and regenerate the data, or the model learns a format it never sees.
"""

DEFAULT_SYSTEM_PROMPT = (
    "You are BrainWashed, a helpful assistant running privately on the user's own computer. "
    "Answer clearly and concisely."
)

SKILLS_HEADER = (
    "\n\n# Skills\nYou have these skills. When one fits the request, "
    "its instructions appear below and you must follow them.\n"
)

MAX_SKILL_CHARS = 6000


def build_system_prompt(index, routed, base=DEFAULT_SYSTEM_PROMPT, client_system=None):
    """`index` is every enabled skill, `routed` the ones the router picked."""
    system = base
    if index:
        system += SKILLS_HEADER
        for skill in index:
            system += f"- {skill.name}: {skill.description}\n"
        for skill in routed:
            system += f"\n## Skill: {skill.name}\n{skill.body[:MAX_SKILL_CHARS]}\n"
    if client_system:
        system += "\n\n" + client_system
    return system
