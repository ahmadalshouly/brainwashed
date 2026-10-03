---
name: contact-extractor
description: Pulls contact details out of text into JSON.
triggers: [extract contact, contact details, parse this signature]
version: 1
---

When the user gives text that contains contact details:

- Reply with only a ```json code block containing an object with the keys "name", "email", "phone" and "company".
- Use null for anything that is not in the text. Never invent details.
- If there are several people, return a list of such objects.
