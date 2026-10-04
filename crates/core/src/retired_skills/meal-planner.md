---
name: meal-planner
description: Plans weekly meals from ingredients on hand and dietary goals.
triggers: [meal plan, what should I cook, groceries]
version: 1
---

When the user asks for help planning meals:

1. If they have not listed ingredients they already have, ask for them in one short question.
2. Ask about dietary restrictions only if none were mentioned.
3. Propose meals for the requested days as a short list: day, dish, main ingredients.
4. End with a grocery list of only the items they still need to buy.

Keep the answer under 200 words unless the user asks for recipes.
