#!/bin/sh
# Post-install hook for the ministr .deb / .rpm packages.
#
# Goal: parity with the macOS .pkg postinstall — the user gets a working
# `ministr` command the moment the installer finishes, no terminal step.
# We symlink the bundled CLI sidecar onto PATH at /usr/local/bin/ministr.
#
# Non-clobbering, exactly like the macOS installer: never overwrite a
# foreign file at that path (a distro package, a manual install, a
# Homebrew-on-Linux shim); only refresh a symlink we ourselves created.
#
# Best-effort: if the sidecar can't be located the script still exits 0 —
# the app's first-launch setup wizard wires PATH on its own as a fallback.
set -e

LINK=/usr/local/bin/ministr

# Tauri lays the externalBin sidecar down next to / near the main binary
# depending on packager. Probe the realistic locations rather than hard-code
# one, so this keeps working if the bundler layout shifts. The SAME list is
# reused below to recognise a symlink we created in a prior version even if
# the layout moved between releases.
PROBE_LOCATIONS="\
/usr/bin/ministr-cli \
/usr/lib/ministr/ministr-cli \
/usr/lib/ministr/binaries/ministr-cli \
/opt/ministr/ministr-cli"

TARGET=""
for c in $PROBE_LOCATIONS; do
  if [ -x "$c" ]; then
    TARGET="$c"
    break
  fi
done

if [ -z "$TARGET" ]; then
  echo "ministr installer: CLI sidecar not found; the app will wire PATH on first launch." >&2
  exit 0
fi

mkdir -p /usr/local/bin

if [ -L "$LINK" ]; then
  current=$(readlink "$LINK")
  # Owned-by-installer if it points at the new target OR at any known
  # ministr bundle location (a prior version's layout). Refresh those to
  # the current TARGET; only a symlink to something outside our probe set
  # is treated as foreign and left alone.
  owned=0
  [ "$current" = "$TARGET" ] && owned=1
  if [ "$owned" -eq 0 ]; then
    for c in $PROBE_LOCATIONS; do
      if [ "$current" = "$c" ]; then
        owned=1
        break
      fi
    done
  fi
  if [ "$owned" -eq 1 ]; then
    ln -sf "$TARGET" "$LINK"
  else
    echo "ministr installer: leaving existing symlink at $LINK unchanged (points to $current, not a ministr bundle)." >&2
  fi
elif [ -e "$LINK" ]; then
  echo "ministr installer: leaving existing file at $LINK unchanged (not created by this installer)." >&2
else
  ln -s "$TARGET" "$LINK"
fi

exit 0
