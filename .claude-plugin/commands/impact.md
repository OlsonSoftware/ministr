---
description: Show the transitive blast radius of changing a symbol — every caller / implementor / importer N levels deep, plus a risk score. Uses ministr_impact.
---

Show the blast radius for changing: $ARGUMENTS

Use the `ministr_impact` MCP tool. If the input is a symbol ID, pass it as `symbol_id`. Otherwise resolve via `ministr_symbols` first.

The response includes a transitive caller list capped at depth 3 by default, the distinct files and modules touched, the count of tests that would need to run, and a risk score (low / medium / high). Review the risk score before recommending the change — high-risk changes warrant breaking the work into smaller, independently-reviewable steps.
