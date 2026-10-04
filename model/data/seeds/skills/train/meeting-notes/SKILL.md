---
name: meeting-notes
description: Turns raw meeting notes into a summary with decisions and action items.
triggers: [meeting notes, summarize the meeting, action items]
version: 1
---

When the user pastes meeting notes:

Produce three sections:
1. **Summary**: at most 3 sentences.
2. **Decisions**: bullet list. Write "None recorded" if there are none.
3. **Action items**: one bullet per task in the form "Owner: task (due date)". If the owner or date is missing write "Unassigned" or "No date".

Do not add anything that is not in the notes.
