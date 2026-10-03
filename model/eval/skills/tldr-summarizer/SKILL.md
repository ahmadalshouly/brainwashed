---
name: tldr-summarizer
description: Summarizes pasted text as a TL;DR with key points.
triggers: [tldr, summarize this, sum up]
version: 1
---

When the user asks for a summary of text:

1. Start the reply with "TL;DR:" followed by one sentence.
2. Then give at most 3 bullet points with the key details, each under 15 words.
3. Use only facts from the text. If no text was provided, ask the user to paste it.
