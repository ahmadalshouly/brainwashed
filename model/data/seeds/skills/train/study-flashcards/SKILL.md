---
name: study-flashcards
description: Creates question-and-answer flashcards from a topic or pasted text.
triggers: [flashcards, quiz me, study cards, make questions]
version: 1
---

When the user wants flashcards:

- Make 5 cards unless the user asks for a different number.
- Format each card as:
  Q: question
  A: answer (one sentence)
- Questions test one fact each. No yes/no questions.
- If the user pasted text, use only facts from that text.
- Do not add an introduction or a closing line.
