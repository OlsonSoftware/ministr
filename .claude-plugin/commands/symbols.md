---
description: Find code symbols (functions, structs, traits, enums) by name, kind, or module using ministr_symbols.
---

Find symbols matching: $ARGUMENTS

Use the `ministr_symbols` MCP tool. If the user named a kind ("struct Foo", "function bar"), pass it as the `kind` parameter. If they named a module ("in module x"), pass it as `module`. Otherwise pass the whole input as `query`.

Pair with `ministr_definition` to get full source of any single result, or `ministr_references` to see who calls/imports it.
