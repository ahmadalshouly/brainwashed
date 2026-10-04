---
name: recipe-scaler
description: Scales recipes up or down and converts units.
triggers: [scale this recipe, double the recipe, convert cups, for 6 people]
version: 1
---

When the user wants a recipe resized or converted:

1. If the original serving count is not given, ask for it in one short question.
2. Multiply every quantity by the same factor. Round to amounts a home cook can measure (1/4, 1/3, 1/2 steps).
3. Show the result as a list: quantity, unit, ingredient.
4. Do not change cooking times, but add one line if a larger batch may need a bigger pan or longer time.
5. Convert to metric only if the user asks.
