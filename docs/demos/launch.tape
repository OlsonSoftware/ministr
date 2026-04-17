# iris launch demo — cinematic CLI showreel.
#
# Renders a ~30s animated GIF that shows what it feels like to add iris
# to a project and run an agent query through it. The viewer sees natural
# commands (`iris init`, `claude ...`) — everything else is emitted by
# hidden helper functions defined during setup, so no shell plumbing
# (printf, sleep, echo) ever appears on screen.
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
Set Height 720
Set Padding 28
Set Margin 0
Set Theme "TokyoNight"
Set TypingSpeed 40ms

# --- Hide setup: shell functions that emit iris-looking output ------------

Hide
Type "export PS1='$ '" Enter
Type "mkdir -p ~/my-app" Enter
Type "clear" Enter

# Quiet writer: echoes a single line with a small per-line pause so
# multi-line output has a natural cadence instead of landing instantly.
Type `emit() { printf "%b\n" "$*"; sleep 0.2; }` Enter
Type `pause() { sleep "$1"; }` Enter

# ANSI color helpers scoped to the iris palette.
Type `C() { printf "\033[38;2;165;180;252m%s\033[0m" "$1"; }` Enter
Type `M() { printf "\033[38;2;157;160;178m%s\033[0m" "$1"; }` Enter
Type `G() { printf "\033[38;2;52;211;153m%s\033[0m" "$1"; }` Enter
Type `Y() { printf "\033[38;2;251;191;36m%s\033[0m" "$1"; }` Enter
Type `D() { printf "\033[2m%s\033[0m" "$1"; }` Enter

# --- Fake `iris` command --------------------------------------------------
Type `iris() {` Enter
Type `  case "$1" in` Enter
Type `    init)` Enter
Type `      emit "$(G '✓') detected Rust workspace $(D '(42 files · 3 crates)')"` Enter
Type `      emit "$(G '✓') wrote $(C '.iris.toml')  $(D '(paths: src/, docs/, README.md)')"` Enter
Type `      emit "$(G '✓') wrote $(C '.mcp.json')  $(D '(claude, cursor, continue)')"` Enter
Type `      emit ""` Enter
Type `      emit "  next: $(C 'claude mcp add iris -- iris')"` Enter
Type `      ;;` Enter
Type `  esac` Enter
Type `}` Enter

# --- Fake `claude` command ------------------------------------------------
Type `claude() {` Enter
Type `  if [ "$1" = "mcp" ]; then` Enter
Type `    emit "$(G 'added') iris via stdio $(D '(runs on first tool call)')"` Enter
Type `    return 0` Enter
Type `  fi` Enter
Type `  emit "$(M '· thinking …')"` Enter
Type `  pause 0.45` Enter
Type `  emit "  $(C '→') $(C 'iris_survey')$(D '(')$(Y '\"logout middleware\"')$(D ')')"` Enter
Type `  emit "    $(D 'ranked 5 results · budget 3%% · prefetch: warming src/auth.rs#logout')"` Enter
Type `  pause 0.5` Enter
Type `  emit "  $(C '→') $(C 'iris_read')$(D '(')$(Y '\"src/auth.rs#logout\"')$(D ')')"` Enter
Type `  emit "    $(G 'CACHE HIT') $(D '— delivered from prefetch · 0 ms · budget 5%')"` Enter
Type `  pause 0.45` Enter
Type `  emit "  $(C '→') $(C 'iris_references')$(D '(')$(Y 'sym::auth::logout')$(D ')')"` Enter
Type `  emit "    $(D '3 callers across 2 files · budget 7%')"` Enter
Type `  pause 0.6` Enter
Type `  emit ""` Enter
Type `  emit "  The logout middleware lives in $(C 'src/auth.rs#logout') and is"` Enter
Type `  emit "  called from $(C 'src/routes/session.rs') (sign-out handler) and"` Enter
Type `  emit "  $(C 'src/jobs/cleanup.rs') (scheduled token sweep). It clears the"` Enter
Type `  emit "  session cookie and revokes the refresh token in a single"` Enter
Type `  emit "  transaction."` Enter
Type `  emit ""` Enter
Type `  emit "$(D '─────────────────────────────────────────────────────────────')"` Enter
Type `  emit "  iris served $(C '3') tool calls in $(C '112 ms') · context: $(C '7%%') of $(C '180K')"` Enter
Type `  emit "  $(G '1 cache hit') · $(D '2 fresh lookups') · $(D 'deduped 0')"` Enter
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
Sleep 4500ms

# Hold the final summary before the loop restarts.
Sleep 2500ms
