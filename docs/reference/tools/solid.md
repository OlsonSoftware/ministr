# ministr_solid

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Detect possible SOLID-principle violations across the codebase deterministically. Returns clusters / findings labelled by principle (dry_ocp, srp, isp, dip). Filter by kind/module and toggle principles via params. Pair with ministr_references before refactoring.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `container_kinds` | array of string | no | SRP container kinds. |
| `cursor` | string | no | Cursor. |
| `cyclic_min_edges_per_direction` | integer | no | Cycle edges per direction. |
| `cyclic_skip_test_paths` | boolean | no | Ignore test/fixture cycle edges. |
| `interface_kinds` | array of string | no | Interface kinds. |
| `isp_max_overlap_fraction` | number | no | ISP usage cutoff. |
| `isp_min_methods` | integer | no | ISP minimum method count. |
| `jaccard_threshold` | number | no | Callee Jaccard threshold. |
| `kind` | string | no | Symbol-kind filter. |
| `limit` | integer | no | Finding limit (cap 500). |
| `max_pairs` | integer | no | Comparison cap. |
| `min_lines` | integer | no | Minimum symbol lines. |
| `module` | string | no | Module-path prefix. |
| `offset` | integer | no | Offset. |
| `principles` | array of string | no | Principles: dry_ocp, srp, isp, dip, shotgun_surgery, cyclic_dependency; empty runs all. |
| `project` | string | no | Linked project. |
| `representative_count` | integer | no | Members per group; excess omitted. |
| `shotgun_max_jaccard` | number | no | Shotgun maximum callee Jaccard. |
| `shotgun_min_packages` | integer | no | Shotgun minimum packages. |
| `shotgun_min_sites` | integer | no | Shotgun minimum sites. |
| `shotgun_skip_conventional_names` | boolean | no | Skip conventional Shotgun names. |
| `similarity_threshold` | number | no | Clone cosine threshold. |
| `srp_cohesion_threshold` | number | no | SRP cohesion threshold. |

## Output

| Field | Type | Description |
|---|---|---|
| `coherence_alerts` | array | Pending coherence alerts (present when underlying content has changed). |
| `completeness` | any | Whether absence is conclusive for the index generation queried. |
| `corpora` | array | Per-corpus status for fan-out/routed operations. |
| `error` | … | Stable error detail for partial/error responses. |
| `indexing_in_progress` | boolean | True when background corpus ingestion is still running. |
| `indexing_message` | string | Human-readable ingestion status message (e.g. "Checking 12/42 files"). |
| `next_actions` | array | Concrete next-tool-call suggestions, in priority order.  Coherence-driven (re-read changed sections) plus any per-handler hints (e.g. survey's top-result follow-up). Budget pressure no longer contributes entries here — see the struct-level note. |
| `result` | … | The tool-specific result data (varying — placed last for prefix stability). |
| `status` | any | Machine-readable operation status; empty successful results remain `ok`. |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
