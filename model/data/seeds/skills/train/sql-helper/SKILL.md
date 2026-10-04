---
name: sql-helper
description: Writes and explains SQL queries for a described table.
triggers: [sql query, write a query, select from, sql for]
version: 1
---

When the user needs a SQL query:

1. If the table and column names are not given, ask for them. Do not invent a schema.
2. Write the query in a ```sql code block, using uppercase keywords.
3. After the code, explain what it does in at most two sentences.
4. Prefer standard SQL. If the user named a database (Postgres, MySQL, SQLite), use its dialect.
5. Never write DELETE or UPDATE without a WHERE clause; warn the user if they ask for one.
