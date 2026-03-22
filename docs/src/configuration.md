# Configuration

iris uses a two-level configuration system: global settings and per-corpus settings.

## Global Configuration

The global config file lives at `~/.iris/config.toml`. All fields are optional and fall back to sensible defaults.

```toml
# Root data directory (default: ~/.iris)
data_dir = "~/.iris"

# Default embedding model for new corpora
default_model = "all-MiniLM-L6-v2"

# Log output format: "pretty" or "json"
log_format = "pretty"

# Default context budget in tokens for new sessions
default_context_budget = 100000

[prefetch]
# Whether speculative prefetching is enabled
enabled = true

# Maximum items in the prefetch cache
cache_size = 50

# Number of recent sections for topical prefetch vector
topic_window = 5
```

### Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `data_dir` | path | `~/.iris` | Root directory for all iris data |
| `default_model` | string | `"all-MiniLM-L6-v2"` | Embedding model for new corpora (see [supported models](#supported-embedding-models)) |
| `log_format` | string | `"pretty"` | Log format: `"pretty"` or `"json"` |
| `default_context_budget` | integer | `100000` | Token budget for new sessions |
| `prefetch.enabled` | boolean | `true` | Enable speculative prefetching |
| `prefetch.cache_size` | integer | `50` | Max prefetch cache entries |
| `prefetch.topic_window` | integer | `5` | Recent sections for topic vector |

## Corpus Configuration

Each corpus has its own config at `~/.iris/corpora/<name>/meta.toml`.

```toml
# Human-readable corpus name
name = "my-project-docs"

# Source directories to index
source_dirs = ["./docs", "./api-reference"]

# Embedding model override (falls back to global default)
model = "bge-small-en-v1.5"

# Watch source directories for changes
watch = true

# Claim extraction mode: "heuristic" or "model_assisted"
claim_extraction = "heuristic"

# Override parser for all files (omit for auto-detection)
# parser = "markdown"
```

### Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | string | `""` | Human-readable corpus name |
| `source_dirs` | list of paths | `[]` | Directories to index |
| `model` | string or null | `null` | Embedding model override |
| `watch` | boolean | `false` | Enable file watching for coherence |
| `claim_extraction` | string | `"heuristic"` | `"heuristic"` or `"model_assisted"` |
| `parser` | string or null | `null` | Force parser: `"markdown"`, `"html"`, or `"pdf"` |

### Parser Auto-Detection

When `parser` is not set, iris detects the parser from file extensions:

| Extension | Parser |
|---|---|
| `.md`, `.markdown` | Markdown (comrak) |
| `.html`, `.htm` | HTML (scraper) |
| `.pdf` | PDF (pdf-extract) |

## CLI Arguments

The `iris` binary accepts these arguments:

```
iris [OPTIONS]

Options:
  -c, --corpus <PATH>    Path to the corpus directory to serve
  -C, --config <PATH>    Path to config file (default: ~/.iris/config.toml)
  -h, --help             Print help
  -V, --version          Print version
```

CLI arguments override config file values. If `--corpus` is provided, iris indexes that directory directly without requiring a pre-configured corpus.

## Supported Embedding Models

iris supports the following embedding models via `fastembed`. Quantized variants (suffix `-q`) use INT8 quantization for faster inference and smaller model files at a slight quality trade-off.

| Model Name | Dimensions | Notes |
|---|---|---|
| `all-MiniLM-L6-v2` | 384 | Default — fast, general-purpose |
| `all-MiniLM-L6-v2-q` | 384 | Quantized variant |
| `all-MiniLM-L12-v2` | 384 | Slightly higher quality |
| `all-MiniLM-L12-v2-q` | 384 | Quantized variant |
| `bge-small-en-v1.5` | 384 | BAAI small English |
| `bge-small-en-v1.5-q` | 384 | Quantized variant |
| `bge-base-en-v1.5` | 768 | BAAI base English |
| `bge-base-en-v1.5-q` | 768 | Quantized variant |
| `bge-large-en-v1.5` | 1024 | BAAI large English |
| `bge-large-en-v1.5-q` | 1024 | Quantized variant |

To use a quantized model, set `default_model` in `config.toml`:

```toml
default_model = "all-MiniLM-L6-v2-q"
```

## Environment Variables

| Variable | Description |
|---|---|
| `RUST_LOG` | Controls log verbosity via `tracing-subscriber::EnvFilter`. Example: `RUST_LOG=iris_core=debug,iris_mcp=info` |
