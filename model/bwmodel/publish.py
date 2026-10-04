"""Hugging Face publishing helpers for the Kaggle notebooks: finding the token,
deciding whether a run is good enough to publish, and writing the cards."""

import os

REPO_URL = "https://github.com/ahmadalshouly/brainwashed"


def hf_token(name="HF_TOKEN"):
    """Reads the Hugging Face token from Kaggle Secrets, falling back to an
    environment variable. Returns None when neither is set."""
    try:
        from kaggle_secrets import UserSecretsClient

        token = UserSecretsClient().get_secret(name)
        if token:
            return token
    except Exception:
        pass  # not on Kaggle, or the secret is not attached to this notebook
    return os.environ.get(name) or None


def should_publish(reports, only_if_better=True):
    """Returns (publish, reason) given the base and fine-tuned eval summaries."""
    base, tuned = reports.get("base"), reports.get("fine-tuned")
    if not only_if_better:
        return True, "publishing without the quality check (PUBLISH_ONLY_IF_BETTER is False)"
    if not base or not tuned:
        return False, "no eval results, so there is nothing to compare"
    if tuned["pass_rate"] <= base["pass_rate"]:
        return False, (
            f"the fine-tune passed {tuned['pass_rate']:.0%} of eval cases, not more than the base "
            f"model's {base['pass_rate']:.0%}; set PUBLISH_ONLY_IF_BETTER = False to publish anyway"
        )
    return True, f"the fine-tune passed {tuned['pass_rate']:.0%} vs {base['pass_rate']:.0%} for the base model"


def eval_table(reports):
    base, tuned = reports.get("base"), reports.get("fine-tuned")
    if not base or not tuned:
        return "No eval results were recorded for this upload."
    rows = ["| Held-out eval | Base model | This model |", "| --- | --- | --- |"]
    rows.append(f"| **All 40 cases** | {base['pass_rate']:.0%} | **{tuned['pass_rate']:.0%}** |")
    for category in sorted(set(base["by_category"]) | set(tuned["by_category"])):
        b = base["by_category"].get(category)
        t = tuned["by_category"].get(category)
        rows.append(f"| {category} | {_pct(b)} | {_pct(t)} |")
    return "\n".join(rows)


def _pct(value):
    return "n/a" if value is None else f"{value:.0%}"


def _about(base_model, data_repo):
    data = f"[{data_repo}](https://huggingface.co/datasets/{data_repo})" if data_repo else "synthetic conversations"
    return (
        f"Fine-tuned from [{base_model}](https://huggingface.co/{base_model}) with LoRA on {data} "
        f"written by an open-weight teacher model, using the host's exact prompt format. Only the language "
        f"model was trained; the vision encoder is unchanged. Training code: [model/]({REPO_URL}/tree/main/model)."
    )


def _what_it_does():
    return (
        "The default model for [BrainWashed](" + REPO_URL + "), an open source app that turns a laptop into a "
        "private AI server. It is trained to:\n\n"
        "- follow `SKILL.md` skills that BrainWashed injects into its prompt, and ignore ones that don't fit;\n"
        "- call tools only when needed, with valid JSON arguments, and ask when a required detail is missing;\n"
        "- answer concisely, so it stays fast on 8 GB laptops."
    )


def gguf_card(name, base_model, reports, weights_repo=None, data_repo=None, multimodal=True):
    vision = f" --mmproj mmproj-{name}-f16.gguf" if multimodal else ""
    files = [
        f"| `{name}-Q4_K_M.gguf` | Recommended. Fits laptops with 8 GB of memory |",
        f"| `{name}-Q8_0.gguf` | Higher quality, about twice the size |",
    ]
    if multimodal:
        files.append(f"| `mmproj-{name}-f16.gguf` | Vision encoder, needed for image input |")
    source = f"\nFull-precision weights: [{weights_repo}](https://huggingface.co/{weights_repo}).\n" if weights_repo else ""
    return f"""---
license: apache-2.0
base_model: {base_model}
library_name: gguf
tags: [gguf, llama.cpp, brainwashed, tool-calling, skills{', multimodal' if multimodal else ''}]
---

# {name} (GGUF)

{_what_it_does()}

## Files

| File | Use |
| --- | --- |
{chr(10).join(files)}
{source}
## Run it

In BrainWashed, add it from the model list. With llama.cpp directly:

```sh
llama-server -m {name}-Q4_K_M.gguf{vision} --jinja
```

`--jinja` is required for tool calling.

## Evaluation

Scored by rules on skills and tools the model never saw in training, both models run as Q4_K_M GGUF:

{eval_table(reports)}

## Training

{_about(base_model, data_repo)}

## License

Apache-2.0, the same as the base model.
"""


def weights_card(name, base_model, reports, gguf_repo=None, data_repo=None):
    gguf = f"\nGGUF files for llama.cpp and BrainWashed: [{gguf_repo}](https://huggingface.co/{gguf_repo}).\n" if gguf_repo else ""
    return f"""---
license: apache-2.0
base_model: {base_model}
library_name: transformers
tags: [brainwashed, tool-calling, skills]
---

# {name}

{_what_it_does()}
{gguf}
## Evaluation

{eval_table(reports)}

## Training

{_about(base_model, data_repo)}

## License

Apache-2.0, the same as the base model.
"""


def dataset_card(teacher, counts, num_skills, num_tools):
    rows = "\n".join(f"| {k} | {v} |" for k, v in sorted(counts.items()))
    return f"""---
license: apache-2.0
task_categories: [text-generation]
tags: [brainwashed, tool-calling, skills, synthetic]
---

# BrainWashed training data

Synthetic conversations used to fine-tune the default [BrainWashed]({REPO_URL}) model. Each row is one
conversation in the OpenAI chat format (`messages`, plus `tools` when tools were offered), built with the
host's exact system prompt: a skill index, the 1-2 routed `SKILL.md` bodies, and tool definitions.

Replies were written by `{teacher}`, an open-weight teacher model, and filtered for valid tool calls, length and
instruction leaks. Skills: {num_skills}. Tools: {num_tools}.

| Category | Conversations |
| --- | --- |
{rows}

Generator: [model/data/generate.py]({REPO_URL}/blob/main/model/data/generate.py). The eval skills and tools in
`model/eval/` are kept out of this data on purpose.
"""
