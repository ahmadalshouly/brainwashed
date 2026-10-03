"""Turns dataset rows into (prompt, completion) text pairs for training.

Each conversation becomes one example per assistant turn: the prompt is
everything before that turn rendered with the model's own chat template and a
generation prompt, and the completion is exactly what the model must produce
next. Loss is only taken on the completion, and the split matches what
llama-server feeds the model at inference, whatever the template does with
earlier turns (Qwen3, for instance, drops old `<think>` blocks).
"""


def assistant_turns(row):
    """Yields (messages_before, assistant_message) for every assistant turn."""
    messages = row["messages"]
    for i, message in enumerate(messages):
        if message["role"] == "assistant":
            yield messages[:i], message


def render_pairs(row, apply_chat_template, **template_kwargs):
    """`apply_chat_template(messages, tools=..., add_generation_prompt=..., **kw)`
    must return text (tokenizer.apply_chat_template with tokenize=False).

    Returns (pairs, skipped) where pairs are (prompt, completion) strings and
    skipped counts turns whose rendering did not split cleanly."""
    tools = row.get("tools") or None
    pairs, skipped = [], 0
    for before, message in assistant_turns(row):
        prompt = apply_chat_template(
            before, tools=tools, add_generation_prompt=True, **template_kwargs
        )
        full = apply_chat_template(
            before + [message], tools=tools, add_generation_prompt=False, **template_kwargs
        )
        if not full.startswith(prompt) or len(full) == len(prompt):
            skipped += 1
            continue
        pairs.append((prompt, full[len(prompt) :]))
    return pairs, skipped
