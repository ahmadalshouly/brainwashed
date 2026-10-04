import json
import unittest

from jinja2 import Environment

from bwmodel.format import render_pairs

# A trimmed copy of the Qwen3 chat template: enough to cover tools, tool
# results and the empty <think> block on the final assistant turn.
QWEN3_LIKE = r"""
{%- if tools %}<|im_start|>system
{{ messages[0].content }}

# Tools
<tools>
{%- for tool in tools %}
{{ tool | tojson }}
{%- endfor %}
</tools><|im_end|>
{% else %}<|im_start|>system
{{ messages[0].content }}<|im_end|>
{% endif %}
{%- set ns = namespace(last_query_index=messages|length - 1) %}
{%- for message in messages[::-1] %}{%- set index = (messages|length - 1) - loop.index0 %}
{%- if message.role == "user" and ns.last_query_index == messages|length - 1 %}{%- set ns.last_query_index = index %}{%- endif %}
{%- endfor %}
{%- for message in messages[1:] %}{%- set i = loop.index0 + 1 %}
{%- if message.role == "user" %}<|im_start|>user
{{ message.content }}<|im_end|>
{% elif message.role == "assistant" %}
{%- if i > ns.last_query_index and loop.last %}<|im_start|>assistant
<think>

</think>

{{ message.content }}{% else %}<|im_start|>assistant
{{ message.content }}{% endif %}
{%- for tc in message.tool_calls or [] %}<tool_call>
{"name": "{{ tc.function.name }}", "arguments": {{ tc.function.arguments | tojson }}}
</tool_call>{% endfor %}<|im_end|>
{% elif message.role == "tool" %}<|im_start|>user
<tool_response>
{{ message.content }}
</tool_response><|im_end|>
{% endif %}
{%- endfor %}
{%- if add_generation_prompt %}<|im_start|>assistant
{% endif %}"""

TEMPLATE = Environment().from_string(QWEN3_LIKE)


def apply(messages, tools=None, add_generation_prompt=False):
    return TEMPLATE.render(messages=messages, tools=tools, add_generation_prompt=add_generation_prompt)


class FormatTest(unittest.TestCase):
    def test_one_pair_per_assistant_turn(self):
        tool = {"type": "function", "function": {"name": "get_weather", "description": "d", "parameters": {"type": "object", "properties": {}}}}
        row = {
            "tools": [tool],
            "messages": [
                {"role": "system", "content": "SYS"},
                {"role": "user", "content": "weather in Oslo?"},
                {"role": "assistant", "content": "", "tool_calls": [{"type": "function", "function": {"name": "get_weather", "arguments": {"city": "Oslo"}}}]},
                {"role": "tool", "name": "get_weather", "content": json.dumps({"temp": 3})},
                {"role": "assistant", "content": "It is 3 degrees in Oslo."},
            ],
        }
        pairs, skipped = render_pairs(row, apply)
        self.assertEqual(skipped, 0)
        self.assertEqual(len(pairs), 2)
        (p1, c1), (p2, c2) = pairs
        self.assertTrue(p1.endswith("<|im_start|>assistant\n"))
        self.assertIn('<tool_call>\n{"name": "get_weather", "arguments": {"city": "Oslo"}}\n</tool_call><|im_end|>', c1)
        self.assertIn("<tools>", p1)
        self.assertIn("<tool_response>", p2)
        self.assertEqual(c2, "<think>\n\n</think>\n\nIt is 3 degrees in Oslo.<|im_end|>\n")

    def test_bad_split_is_skipped(self):
        row = {"messages": [{"role": "system", "content": "S"}, {"role": "user", "content": "u"}, {"role": "assistant", "content": "a"}]}
        pairs, skipped = render_pairs(row, lambda m, tools=None, add_generation_prompt=False: "X" if add_generation_prompt else "Y")
        self.assertEqual((pairs, skipped), ([], 1))


if __name__ == "__main__":
    unittest.main()
