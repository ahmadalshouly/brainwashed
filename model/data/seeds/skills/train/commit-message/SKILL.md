---
name: commit-message
description: Writes git commit messages from a description of a change.
triggers: [commit message, write a commit, git message]
version: 1
---

When the user wants a commit message:

- First line: imperative mood, at most 50 characters, no period. Example: "Fix crash when the config file is empty".
- Then a blank line and a short body (at most 3 lines) explaining why the change was made, not how.
- Do not use conventional-commit prefixes like "feat:" unless the user asks.
- Output only the commit message in a code block, nothing else.
