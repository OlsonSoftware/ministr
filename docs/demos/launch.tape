# iris launch demo — cinematic CLI showreel.
#
# Renders a ~35s animated GIF that shows what it feels like to add iris
# to a project and run an agent query through Claude Code. The viewer
# sees natural commands (`iris init`, `claude mcp add iris`, then
# `claude "…"`); the tool call output is styled to match Claude Code's
# actual terminal UI (⏺ tool markers, ⎿ result prefix, rounded summary
# box). All output is emitted by hidden helper functions so no shell
# plumbing (printf, sleep) appears on screen.
#
# Regenerate with:
#   vhs docs/demos/launch.tape
#
# Output: docs/src/assets/launch.gif

Output docs/src/assets/launch.gif

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 18
Set LineHeight 1.35
Set Width 1240
Set Height 780
Set Padding 28
Set Margin 0
Set Theme "TokyoNight"
Set TypingSpeed 40ms

# --- Hide setup: shell state + helper functions ---------------------------

Hide
Type "export PS1='$ '" Enter
Type "mkdir -p ~/my-app" Enter
Type "clear" Enter

# ANSI colour codes exported as vars. Using vars (vs function subshells)
# keeps each emit line readable and avoids quote-escaping hell.
Type `RESET=$'\e[0m'` Enter
Type `DIM=$'\e[2m'` Enter
Type `BOLD=$'\e[1m'` Enter
Type `IRIS=$'\e[38;2;165;180;252m'` Enter
Type `MUTE=$'\e[38;2;157;160;178m'` Enter
Type `GRN=$'\e[38;2;52;211;153m'` Enter
Type `ORA=$'\e[38;2;251;146;60m'` Enter
Type `AMB=$'\e[38;2;251;191;36m'` Enter
Type `CYN=$'\e[38;2;103;232;249m'` Enter

# Emit one line with a small pause so multi-line blocks have cadence.
Type `emit() { printf '%b\n' "$*"; sleep 0.18; }` Enter
Type `pause() { sleep "$1"; }` Enter

# --- Fake `iris` command (scaffolding output) ----------------------------
Type `iris() {` Enter
Type `  case "$1" in` Enter
Type `    init)` Enter
Type `      emit "${GRN}✓${RESET} detected Rust workspace ${DIM}(42 files · 3 crates)${RESET}"` Enter
Type `      emit "${GRN}✓${RESET} wrote ${IRIS}.iris.toml${RESET}  ${DIM}(paths: src/, docs/, README.md)${RESET}"` Enter
Type `      emit "${GRN}✓${RESET} wrote ${IRIS}.mcp.json${RESET}  ${DIM}(claude, cursor, continue)${RESET}"` Enter
Type `      emit ""` Enter
Type `      emit "  next: ${IRIS}claude mcp add iris -- iris${RESET}"` Enter
Type `      ;;` Enter
Type `  esac` Enter
Type `}` Enter

# --- Fake `claude` command (Claude Code terminal UI) ---------------------
Type `claude() {` Enter
Type `  if [ "$1" = "mcp" ]; then` Enter
Type `    emit "${GRN}added${RESET} iris via stdio ${DIM}(runs on first tool call)${RESET}"` Enter
Type `    return 0` Enter
Type `  fi` Enter
Type `  # Claude Code interactive turn` Enter
Type `  emit "${DIM}>${RESET} trace the logout middleware"` Enter
Type `  emit ""` Enter
Type `  pause 0.4` Enter
Type `  emit "${ORA}⏺${RESET} I'll trace the logout middleware across the workspace."` Enter
Type `  emit ""` Enter
Type `  pause 0.5` Enter
Type `  emit "${ORA}⏺${RESET} ${BOLD}iris_survey${RESET}${DIM}(query: \"logout middleware\")${RESET}"` Enter
Type `  emit "  ${DIM}⎿${RESET}  ${GRN}5 results${RESET} · ${DIM}18ms · prefetch warming${RESET} ${IRIS}src/auth.rs#logout${RESET}"` Enter
Type `  pause 0.55` Enter
Type `  emit "${ORA}⏺${RESET} ${BOLD}iris_read${RESET}${DIM}(section_id: \"src/auth.rs#logout\")${RESET}"` Enter
Type `  emit "  ${DIM}⎿${RESET}  ${GRN}CACHE HIT${RESET} ${DIM}— delivered from prefetch · 0ms · 420 tokens${RESET}"` Enter
Type `  pause 0.5` Enter
Type `  emit "${ORA}⏺${RESET} ${BOLD}iris_references${RESET}${DIM}(symbol: \"auth::logout\")${RESET}"` Enter
Type `  emit "  ${DIM}⎿${RESET}  ${GRN}3 callers${RESET} ${DIM}across 2 files${RESET}"` Enter
Type `  pause 0.7` Enter
Type `  emit ""` Enter
Type `  emit "The logout middleware lives in ${IRIS}src/auth.rs${RESET} at ${BOLD}logout()${RESET}."` Enter
Type `  emit "It's called from ${IRIS}src/routes/session.rs${RESET} ${DIM}(sign-out handler)${RESET} and"` Enter
Type `  emit "${IRIS}src/jobs/cleanup.rs${RESET} ${DIM}(scheduled token sweep)${RESET}. It clears the"` Enter
Type `  emit "session cookie and revokes the refresh token in a single"` Enter
Type `  emit "transaction."` Enter
Type `  emit ""` Enter
Type `  pause 0.5` Enter
Type `  emit "${DIM}╭──────────────────────────────────────────────────────────────────╮${RESET}"` Enter
Type `  emit "${DIM}│${RESET} ${IRIS}iris${RESET} served ${BOLD}3${RESET} tool calls · context ${BOLD}7%${RESET} of 180K ${DIM}·${RESET} 112 ms    ${DIM}│${RESET}"` Enter
Type `  emit "${DIM}│${RESET} ${GRN}1 cache hit${RESET} ${DIM}·${RESET} 2 fresh lookups ${DIM}·${RESET} deduped 0 ${DIM}·${RESET} prefetch 2/3  ${DIM}│${RESET}"` Enter
Type `  emit "${DIM}╰──────────────────────────────────────────────────────────────────╯${RESET}"` Enter
Type `}` Enter

Type "clear" Enter
Show

# --- Scene 1: project setup -----------------------------------------------

Type "cd ~/my-app" Sleep 500ms Enter
Type "iris init" Sleep 400ms Enter
Sleep 1800ms

Type "claude mcp add iris -- iris" Sleep 400ms Enter
Sleep 900ms

# --- Scene 2: a real agent turn -------------------------------------------

Type `claude "trace the logout middleware"` Sleep 400ms Enter
Sleep 6500ms

# Hold the final summary before the loop restarts.
Sleep 2500ms
