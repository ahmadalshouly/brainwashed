# BrainWashed model

Training code for the default BrainWashed model: a 2-3B open model fine-tuned to follow injected `SKILL.md` instructions, emit valid tool calls and stay concise on consumer hardware.

Planned layout (Phase 5, see [docs/architecture.md](../docs/architecture.md#5-ahmads-fine-tuned-2-3b-model)):

- `data/`: synthetic training example generation and filtering
- `train/`: QLoRA fine-tuning configs
- `eval/`: skill-following and tool-call evals on held-out skills
- `export/`: merge, convert to GGUF and quantize
