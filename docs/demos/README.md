# Demo Scripts

Recording scripts for iris launch demos. Use [asciinema](https://asciinema.org/) for terminal recordings.

## Setup

```sh
# Install asciinema
brew install asciinema

# Pre-warm: index a demo project so the recording doesn't wait on indexing
cd ~/demo-project
iris init
```

Use a clean, small-to-medium open source Rust project as the demo target (e.g., a ~50-file crate). Avoid this repo to prevent conflicts with the live iris instance.

---

## 60-Second Demo

**Goal**: Show iris replacing grep+cat with semantic, token-efficient codebase navigation.

**Audience**: Developers who use LLM coding agents and feel the pain of context window limits.

### Script

```
# Start recording
asciinema rec iris-60s.cast --title "iris: semantic context for LLM agents"

# 1. Setup (5s)
cd ~/demo-project
# Show project size
find src -name '*.rs' | wc -l
# → "52 Rust files"

# 2. The problem (15s)
# Show what grep+cat gives an agent
grep -rl "error" src/ | head -5
wc -c $(grep -rl "error" src/ | head -5)
# → "~12,000 bytes of raw files just to answer 'how does error handling work?'"

# 3. iris init (10s)
iris init
# → "Indexed 52 files, 480 sections, 1,847 claims in 8s"

# 4. Semantic search (15s)
iris survey --query "how does error handling work"
# → Shows 5-10 ranked sections with relevance scores
# → Total: ~400 tokens vs ~12,000 from grep+cat

# 5. Code navigation (10s)
iris symbols --query "Error" --kind enum
# → Lists all error enums across the project
iris definition --symbol "AppError"
# → Shows the exact definition, nothing else

# 6. Closing (5s)
# "90%+ token savings. Sub-60ms queries. Works with any MCP client."

# Stop recording
exit
```

### Post-Production

```sh
# Upload to asciinema
asciinema upload iris-60s.cast

# Or convert to GIF for embedding
# pip install asciinema-agg
agg iris-60s.cast iris-60s.gif --cols 100 --rows 30
```

---

## 2-Minute Deep Dive

**Goal**: Show the full iris workflow including session tracking, budget management, and prefetch.

**Audience**: Technical users evaluating iris for their team or comparing to other RAG solutions.

### Script

```
# Start recording
asciinema rec iris-deep-dive.cast --title "iris: deep dive — session shadow, budget, prefetch"

# 1. Index a project (15s)
cd ~/demo-project
iris init
# Show what was indexed
iris toc --limit 20

# 2. Semantic search vs grep (20s)
# Grep: exact match only
grep -rn "authenticate" src/
# iris: conceptual understanding
iris survey --query "user authentication flow"
# → iris finds auth middleware, session validation, token refresh
# → grep only finds files with the literal string "authenticate"

# 3. Session deduplication (20s)
# First read — full content delivered
iris read --section "src/auth/middleware.rs::validate_token"
# Second read — deduplicated, returns "already in session"
iris read --section "src/auth/middleware.rs::validate_token"
# → "Section already delivered in this session (0 new tokens)"

# 4. Symbol navigation (20s)
iris symbols --query "validate" --kind function
iris definition --symbol "validate_token"
iris references --symbol "validate_token"
# → Shows all 6 callers across the codebase

# 5. Budget management (15s)
iris budget
# → Shows context budget status, recommendations
iris compress --section "src/auth/middleware.rs::validate_token"
# → Compressed summary: 3 claims, 45 tokens (vs 180 original)

# 6. Extract claims (15s)
iris extract --section "src/auth/mod.rs" --query "security"
# → Atomic claims about security-relevant behavior
# → Each claim is independently addressable and cacheable

# 7. Closing (15s)
# Recap the three pillars:
# - Session shadow: dedup + delta delivery
# - Budget manager: eviction + compression
# - Prefetch engine: anticipates what you need next
# "cargo install iris-cli — works with Claude Code, Cursor, any MCP client"

exit
```

### Post-Production

```sh
asciinema upload iris-deep-dive.cast
agg iris-deep-dive.cast iris-deep-dive.gif --cols 120 --rows 35
```

---

## Recording Tips

- Use a clean terminal with a dark theme and large font (14-16pt)
- Set `PS1='$ '` for a minimal prompt
- Type at a natural pace — too fast looks scripted, too slow loses attention
- Pause briefly (1-2s) after each command output so viewers can read
- If using `asciinema`, you can edit the `.cast` file to adjust timing after recording
- Test the full script once before recording to catch any issues
