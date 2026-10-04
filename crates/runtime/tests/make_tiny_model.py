# Builds a tiny random-weight llama GGUF so the runtime can be tested end to end
# without downloading a real model.
import sys, numpy as np, gguf

def bytes_to_unicode():
    bs = list(range(ord("!"), ord("~")+1)) + list(range(ord("¡"), ord("¬")+1)) + list(range(ord("®"), ord("ÿ")+1))
    cs = bs[:]
    n = 0
    for b in range(256):
        if b not in bs:
            bs.append(b); cs.append(256+n); n += 1
    return [chr(c) for _, c in sorted(zip(bs, cs))]

out = sys.argv[1]
vocab = bytes_to_unicode() + ["<|im_start|>", "<|im_end|>"]
n_vocab, n_embd, n_ff, n_head, n_layer, ctx = len(vocab), 64, 128, 4, 2, 4096
w = gguf.GGUFWriter(out, "llama")
w.add_name("tiny-test")
w.add_context_length(ctx); w.add_embedding_length(n_embd); w.add_block_count(n_layer)
w.add_feed_forward_length(n_ff); w.add_head_count(n_head); w.add_head_count_kv(n_head)
w.add_layer_norm_rms_eps(1e-5); w.add_rope_dimension_count(n_embd // n_head)
w.add_tokenizer_model("gpt2"); w.add_tokenizer_pre("default")
w.add_token_list(vocab); w.add_token_types([1]*256 + [3, 3])
w.add_token_merges(["Ġ t"])
w.add_bos_token_id(256); w.add_eos_token_id(257)
w.add_chat_template("{% for m in messages %}<|im_start|>{{ m.role }}\n{{ m.content }}<|im_end|>\n{% endfor %}{% if add_generation_prompt %}<|im_start|>assistant\n{% endif %}")
rng = np.random.default_rng(0)
def t(name, *shape): w.add_tensor(name, (rng.standard_normal(shape) * 0.02).astype(np.float32))
def ones(name, n): w.add_tensor(name, np.ones(n, dtype=np.float32))
t("token_embd.weight", n_vocab, n_embd); ones("output_norm.weight", n_embd); t("output.weight", n_vocab, n_embd)
for i in range(n_layer):
    ones(f"blk.{i}.attn_norm.weight", n_embd); ones(f"blk.{i}.ffn_norm.weight", n_embd)
    for n in ("attn_q", "attn_k", "attn_v", "attn_output"): t(f"blk.{i}.{n}.weight", n_embd, n_embd)
    t(f"blk.{i}.ffn_gate.weight", n_ff, n_embd); t(f"blk.{i}.ffn_up.weight", n_ff, n_embd); t(f"blk.{i}.ffn_down.weight", n_embd, n_ff)
w.write_header_to_file(); w.write_kv_data_to_file(); w.write_tensors_to_file(); w.close()
print("wrote", out)
