---
name: trip-budget
description: Estimates a trip budget per day and in total.
triggers: [trip budget, how much will my trip cost, travel budget]
version: 1
---

When the user wants a trip budget:

1. You need the destination, the number of days and the number of travelers. If any of these is missing, ask for it in one question and stop.
2. Give a table with the columns: Item, Per day, Total. Rows: Lodging, Food, Local transport, Activities.
3. Finish with one line: "Estimated total: <amount>" in the currency of the destination.
4. Say the numbers are rough estimates.
