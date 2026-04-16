# iris launch demo — cinematic CLI showreel.
#
# Renders a 30-second animated GIF that shows what it feels like to add
# iris to a project and run an agent query through it. All output is
# scripted via `printf` + `sleep` so the tape is deterministic and
# doesn't require a running daemon. Regenerate with:
#
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

# --- Hide setup that creates helper fns used by every line -----------------

Hide
Type "export PS1='%F{iris}$ %f' " Enter
Type "clear" Enter
Type `C() { printf "\033[38;2;165;180;252m%s\033[0m" "$1"; }` Enter
Type `M() { printf "\033[38;2;157;160;178m%s\033[0m" "$1"; }` Enter
Type `G() { printf "\033[38;2;52;211;153m%s\033[0m" "$1"; }` Enter
Type `Y() { printf "\033[38;2;251;191;36m%s\033[0m" "$1"; }` Enter
Type `D() { printf "\033[2m%s\033[0m" "$1"; }` Enter
Type "clear" Enter
Show

# --- Scene 1: project setup -----------------------------------------------

Type "cd ~/my-app" Sleep 500ms Enter
Type "iris init" Sleep 400ms Enter
Sleep 200ms
Type@10ms `printf "$(G '✓') detected Rust workspace $(D '(42 files · 3 crates)')\n"` Enter
Sleep 200ms
Type@10ms `printf "$(G '✓') wrote $(C '.iris.toml')  $(D '(paths: src/, docs/, README.md)')\n"` Enter
Sleep 200ms
Type@10ms `printf "$(G '✓') wrote $(C '.mcp.json')  $(D '(claude, cursor, continue)')\n"` Enter
Sleep 300ms
Type@10ms `printf "\n  next: $(C 'claude mcp add iris -- iris')\n\n"` Enter

Sleep 900ms

Type "claude mcp add iris -- iris" Sleep 400ms Enter
Sleep 150ms
Type@10ms `printf "$(G 'added') iris via stdio $(D '(runs on first tool call)')\n\n"` Enter

Sleep 700ms

# --- Scene 2: a real agent turn -------------------------------------------

Type `claude "trace the logout middleware"` Sleep 400ms Enter
Sleep 300ms

# Agent reasoning step
Type@18ms `printf "$(M '· thinking …')\n"` Enter
Sleep 450ms

# Tool call 1
Type@8ms `printf "  $(C '→') $(C 'iris_survey')$(D '(')$(Y '\"logout middleware\"')$(D ')')\n"` Enter
Type@8ms `printf "    $(D 'ranked 5 results · budget 3%% · prefetch: warming src/auth.rs#logout')\n"` Enter
Sleep 500ms

# Tool call 2
Type@8ms `printf "  $(C '→') $(C 'iris_read')$(D '(')$(Y '\"src/auth.rs#logout\"')$(D ')')\n"` Enter
Type@8ms `printf "    $(G 'CACHE HIT') $(D '— delivered from prefetch · 0 ms · budget 5%%')\n"` Enter
Sleep 450ms

# Tool call 3
Type@8ms `printf "  $(C '→') $(C 'iris_references')$(D '(')$(Y 'sym::auth::logout')$(D ')')\n"` Enter
Type@8ms `printf "    $(D '3 callers across 2 files · budget 7%%')\n"` Enter
Sleep 600ms

# Agent answer
Type@12ms `printf "\n  The logout middleware lives in $(C 'src/auth.rs#logout') and is\n"` Enter
Type@12ms `printf "  called from $(C 'src/routes/session.rs') (sign-out handler) and\n"` Enter
Type@12ms `printf "  $(C 'src/jobs/cleanup.rs') (scheduled token sweep). It clears the\n"` Enter
Type@12ms `printf "  session cookie and revokes the refresh token in a single\n"` Enter
Type@12ms `printf "  transaction.\n\n"` Enter

Sleep 600ms

# --- Scene 3: summary line -----------------------------------------------

Type@8ms `printf "$(D '─────────────────────────────────────────────────────────────')\n"` Enter
Type@8ms `printf "  iris served $(C '3') tool calls in $(C '112 ms') · context: $(C '7%%') of $(C '180K')\n"` Enter
Type@8ms `printf "  $(G '1 cache hit') · $(D '2 fresh lookups') · $(D 'deduped 0')\n"` Enter

Sleep 2500ms
