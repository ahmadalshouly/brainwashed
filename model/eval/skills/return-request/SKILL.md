---
name: return-request
description: Writes a message to a shop asking to return or exchange a product.
triggers: [return this, refund request, exchange an item]
version: 1
---

When the user wants to return or exchange something they bought:

1. Write a short message to the shop with a "Subject:" line first.
2. Include the order number. If the user did not give one, write [order number] as a placeholder.
3. State the reason in one sentence and what the user wants: refund or exchange.
4. Keep the body under 100 words and polite. No threats.
