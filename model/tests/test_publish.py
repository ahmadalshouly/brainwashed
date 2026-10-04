import os
import unittest
from unittest import mock

import yaml

from bwmodel.publish import dataset_card, gguf_card, hf_token, should_publish, weights_card

REPORTS = {
    "base": {"pass_rate": 0.45, "by_category": {"tool_call": 0.5, "skill_follow": 0.4}},
    "fine-tuned": {"pass_rate": 0.8, "by_category": {"tool_call": 0.9, "skill_follow": 0.75}},
}


def front_matter(card):
    assert card.startswith("---\n")
    return yaml.safe_load(card.split("---\n")[1])


class PublishTest(unittest.TestCase):
    def test_publishes_only_when_better(self):
        self.assertTrue(should_publish(REPORTS)[0])
        worse = {"base": REPORTS["fine-tuned"], "fine-tuned": REPORTS["base"]}
        ok, reason = should_publish(worse)
        self.assertFalse(ok)
        self.assertIn("PUBLISH_ONLY_IF_BETTER", reason)
        self.assertTrue(should_publish(worse, only_if_better=False)[0])
        self.assertFalse(should_publish({})[0])

    def test_gguf_card(self):
        card = gguf_card("brainwashed-2b", "Qwen/Qwen3.5-2B", REPORTS, "me/brainwashed-2b", "me/data")
        meta = front_matter(card)
        self.assertEqual(meta["license"], "apache-2.0")
        self.assertEqual(meta["base_model"], "Qwen/Qwen3.5-2B")
        self.assertIn("multimodal", meta["tags"])
        self.assertIn("--mmproj mmproj-brainwashed-2b-f16.gguf --jinja", card)
        self.assertIn("| **All 40 cases** | 45% | **80%** |", card)
        self.assertIn("https://huggingface.co/datasets/me/data", card)
        text_only = gguf_card("x", "Qwen/Qwen3-1.7B", REPORTS, multimodal=False)
        self.assertNotIn("mmproj", text_only)
        self.assertNotIn("multimodal", front_matter(text_only)["tags"])

    def test_weights_and_dataset_cards(self):
        meta = front_matter(weights_card("brainwashed-2b", "Qwen/Qwen3.5-2B", REPORTS, "me/brainwashed-2b-GGUF"))
        self.assertEqual(meta["library_name"], "transformers")
        card = dataset_card("unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF", {"tool_call": 10, "no_skill": 3}, 162, 92)
        self.assertEqual(front_matter(card)["license"], "apache-2.0")
        self.assertIn("| tool_call | 10 |", card)

    def test_token_from_environment(self):
        with mock.patch.dict(os.environ, {"HF_TOKEN": "hf_test"}):
            self.assertEqual(hf_token(), "hf_test")
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertIsNone(hf_token())


if __name__ == "__main__":
    unittest.main()
