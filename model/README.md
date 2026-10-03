# BrainWashed model

Everything needed to train the default BrainWashed model on a free Kaggle GPU: a small, multimodal, Apache-2.0 model fine-tuned to follow `SKILL.md` skills and make clean tool calls, shipped as GGUF for the host's llama.cpp runtime.

Skills are still injected into the prompt at runtime (see [docs/architecture.md](../docs/architecture.md#4-skills-teach-it-with-a-markdown-file)). Training does not bake any skill in. It teaches the model the *habit* of reading whatever skill the host routes to it and obeying it, ignoring skills that don't fit, and calling tools only when needed, with valid arguments.

## Layout

| Path | What it is |
|---|---|
| `notebooks/01_generate_data.ipynb` | Kaggle notebook: runs a teacher model and writes the training data |
| `notebooks/02_train.ipynb` | Kaggle notebook: LoRA fine-tune, merge, GGUF export, eval, optional Hugging Face upload |
| `data/generate.py` | The data generator (works with any OpenAI-compatible teacher) |
| `data/seeds/` | Seed skills, tool definitions and skill topics the generator starts from |
| `eval/` | Held-out skills, tools and 40 rule-checked test cases, plus `run_eval.py` |
| `bwmodel/` | Shared code: the host's prompt format, skill parsing, tool-call checks, chat-template rendering |
| `tests/` | Unit and end-to-end tests (no GPU or network needed) |

## Train it on Kaggle, step by step

1. **Kaggle account.** Sign up at kaggle.com and verify your phone number (Settings → Phone verification). Without it, notebooks get no GPU and no internet.
2. **Import the notebooks.** On Kaggle: Create → New Notebook → File → Import Notebook, and upload `model/notebooks/01_generate_data.ipynb`. Do the same for `02_train.ipynb`.
3. **Generate the data** with `01_generate_data`:
   - In the right panel set Accelerator to **GPU T4 x2** and Internet to **On**.
   - Leave the defaults to run the teacher (Qwen3-30B-A3B-Instruct-2507, Apache-2.0) on Kaggle's GPUs for free. Or set `USE_HOSTED_TEACHER = True` and add your API key under Add-ons → Secrets as `TEACHER_API_KEY`.
   - Click **Save Version → Save & Run All (Commit)**. It keeps running after you close the tab. Building llama.cpp and downloading the teacher take about 20 minutes; generation speed depends on the teacher, so start with the default 3,000 conversations and check the log.
4. **Train** with `02_train`:
   - GPU T4 x2, Internet On.
   - **Add Input → Your Work →** your `01_generate_data` notebook.
   - Optional: add a Hugging Face write token as the secret `HF_TOKEN` and set `HF_REPO` to publish the GGUF files.
   - **Save Version → Save & Run All (Commit)**.
5. **Read the result.** The last cells print a table comparing the base model and your fine-tune on the held-out eval, then show the model a picture to confirm it still sees images. Ship it only if the fine-tune wins. The GGUF files are in the notebook's Output tab under `gguf/`: `*-Q4_K_M.gguf` is the one for 8 GB laptops, and `mmproj-*.gguf` is the vision part.
6. **Try it.** `llama-server -m brainwashed-2b-Q4_K_M.gguf --mmproj mmproj-brainwashed-2b-f16.gguf --jinja`, or load it in the BrainWashed host.

Kaggle's free tier gives about 30 GPU hours a week and 12 hours per session. A full run of both notebooks with the defaults should fit in one week's quota.

## Choices made

- **Base model: `Qwen/Qwen3.5-2B`** (Apache-2.0). It is small, reads images and video as well as text, calls tools natively, and runs in llama.cpp with a separate vision file (`--mmproj`). It answers without a thinking phase by default, which suits slow laptops. One line in the training notebook switches to:
  - `Qwen/Qwen3.5-4B`: the same model family, smarter, still fits an 8 GB laptop at Q4 (set `LOAD_IN_4BIT = True`).
  - `google/gemma-4-E2B-it`: Apache-2.0 too, and it also understands audio (voice notes up to 30 seconds), at the cost of a bigger file (5.1B parameters stored, 2.3B active).
- **Only the language part is trained.** The vision and audio encoders stay frozen, so image understanding survives fine-tuning. The training data is text only; image-and-tool examples can be added later once the app sends images.
- **Teacher: an open-weight Qwen3 model.** Its outputs can be used for training. Most closed-model APIs forbid using their outputs to train other models, so check the terms before swapping in a hosted teacher.
- **LoRA (r=16) on all attention and MLP projections**, 2 epochs, learning rate 2e-4, loss only on the assistant's replies.
- **Plain Hugging Face `transformers` + `peft`** rather than a faster wrapper, because it has the fewest moving parts to break on Kaggle. The notebook pins the library versions it was tested with.

## What the data teaches

Every conversation uses the host's real system prompt (`bwmodel/prompt.py` mirrors `Engine::build_prompt` in `crates/core`): the skill index, the 1-2 routed skill bodies, and tools in the OpenAI format, which llama-server renders with the model's own chat template.

| Category | Share | Teaches |
|---|---|---|
| `skill_follow` | 30% | Follow the routed skill's steps and format exactly; ask first when the skill says to and details are missing |
| `skill_two_routed` | 7% | Two skills routed, only one fits: use the right one |
| `skill_wrong_route` | 6% | The router picked a skill that doesn't fit: ignore it |
| `no_skill` | 10% | No skill applies: just answer well |
| `tool_call` | 22% | Call the right tool with valid arguments, then answer from the result (including tool errors) |
| `tool_missing_args` | 6% | A required argument is missing: ask instead of guessing |
| `tool_not_needed` | 8% | Tools are offered but not needed: answer directly |
| `skill_with_tools` | 11% | Follow a skill that involves a tool |

About 20% of conversations get a follow-up turn, about 12% of user messages are in Arabic, Spanish or French, and some use a different base system prompt (users can change it in settings). On top of the 12 seed skills and 32 seed tools, the teacher writes about 150 more skills and 60 more tools so the model learns to read any skill, not memorize a few. Replies are filtered: invalid tool calls, tool calls where none were wanted, leaked instructions and overlong answers are dropped (`rejected.jsonl` says why).

## The eval

`eval/` holds 8 skills and 8 tools that never appear in training (the generator refuses to use their names), and 40 cases checked by rules, not by another model: required lines and labels, word limits, JSON output, asking a question when information is missing, picking the right tool with the right arguments, not calling tools when none is needed, and answering correctly from a tool result.

Run it against any OpenAI-compatible server:

```sh
llama-server -m brainwashed-1.7b-Q4_K_M.gguf --jinja --port 8081 &
python model/eval/run_eval.py --base-url http://127.0.0.1:8081/v1 --out report.json
```

## Running outside Kaggle

Any machine with a 16 GB+ NVIDIA GPU works. Run the notebook cells in order, or use the generator directly against any teacher:

```sh
python model/data/generate.py --base-url http://127.0.0.1:8080/v1 --model teacher --out data --num 3000
```

## Keeping it in sync with the host

If the host's system prompt or skill layout changes (`crates/core/src/settings.rs`, `crates/core/src/skills.rs`), update `bwmodel/prompt.py` and regenerate the data, or the model learns a format it never sees in the app. The host does not send tools or images to the model yet: the tool runner is a later phase, and image input needs the runtime to pass `--mmproj` to llama-server and the apps to attach photos. Training on tool calls now means the model is ready when it lands, and llama-server already returns them as OpenAI `tool_calls`.

## Tests

```sh
cd model && python -m unittest discover -s tests -t .
```

Needs only `pyyaml` and `jinja2`. The tests run the generator and the eval end to end against a fake teacher.
