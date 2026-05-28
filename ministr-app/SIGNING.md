# macOS code signing & notarization

The ministr desktop app and the bundled `ministr` CLI both need to be signed
with a **Developer ID Application** certificate and notarized by Apple
before they can be distributed to end users. Without signing, Gatekeeper
will block launch on anything other than the developer's own machine.

Out of the box this repo builds **unsigned**. Signing is off until an
identity is configured — `ministr-app/src-tauri/tauri.conf.json` has
`bundle.macOS.signingIdentity: null`. Signed + notarized `.pkg`
artifacts are produced in CI (see [Building](#building) below), not from
a local recipe.

This doc captures what's needed to turn signing on.

## One-time setup

1. Enroll in the [Apple Developer Program](https://developer.apple.com/programs/)
   if you haven't already (~$99 / year).
2. In *Keychain Access → Certificates*, request and install two certs
   from your developer account:
   - **Developer ID Application** — signs the `.app` bundle and the CLI
     binary under the hardened runtime.
   - **Developer ID Installer** — signs the outer `.pkg` distribution
     package.
3. Create an app-specific password at [appleid.apple.com](https://appleid.apple.com/)
   under *Security → App-Specific Passwords*. This is what notarytool
   uses — you do **not** use your real Apple ID password.
4. Note your 10-character Team ID — visible in Apple Developer
   membership details.

## Environment variables

The CI `.pkg` builder (`scripts/ci/ci.py pkg`) reads these from the
environment — bound as secrets on the release workflows (see
[CI signing](#ci-signing)):

| Variable                       | Example                                             | Required by                |
| ------------------------------ | --------------------------------------------------- | -------------------------- |
| `APPLE_SIGNING_IDENTITY`           | `Developer ID Application: Your Name (TEAMID)`  | signing (always)        |
| `APPLE_INSTALLER_SIGNING_IDENTITY` | `Developer ID Installer: Your Name (TEAMID)`    | `.pkg` signing (always) |
| `APPLE_ID`                         | `you@example.com`                               | notarization            |
| `APPLE_PASSWORD`                   | app-specific password                           | notarization            |
| `APPLE_TEAM_ID`                    | `ABCD123456`                                    | notarization            |

These are bound as repo secrets on the release workflows, not sourced
from a local file — see [CI signing](#ci-signing).

## Wire up Tauri's bundler

In `ministr-app/src-tauri/tauri.conf.json`, set:

```json
"macOS": {
  "signingIdentity": "Developer ID Application: Your Name (TEAMID)",
  "entitlements": "./Entitlements.plist",
  "minimumSystemVersion": "13.0"
}
```

The entitlements file at `ministr-app/src-tauri/Entitlements.plist` already
grants the three entitlements WKWebView needs under hardened runtime
(`allow-jit`, `allow-unsigned-executable-memory`,
`allow-dyld-environment-variables`). Do not add entitlements beyond what
the app actually uses — Apple scrutinizes broad entitlements.

`signingIdentity` in `tauri.conf.json` is only consulted by the
`.dmg`/`.app` bundler path (`pnpm tauri build`). The `.pkg` path in
`scripts/ci/ci.py pkg` reads the same identity from the env var directly
via `codesign` and `productbuild`.

## Building

Signed + notarized `.pkg` artifacts are built in **CI**, not locally.
Local signed builds were retired in F31: `scripts/build-pkg.sh` and the
`just pkg` / `just pkg-dev` recipes are gone.

The release flow (see [RELEASE.md](../RELEASE.md)):

- A `chore: release vX.Y.Z` PR merged here pushes tag `v<X.Y.Z>` and
  fires a `repository_dispatch` into **ministr-private**'s `release.yml`.
- That workflow clones both repos as siblings, builds the cloud-capable
  `ministr` against the MIT crates here, then runs
  `python3 scripts/ci/ci.py pkg` for the macOS packaging step:
  `pkgbuild` (component + postinstall CLI symlink) → `productbuild`
  (signed) → `notarytool --wait` → `stapler`, and uploads the artifacts
  to this repo's Release via a cross-repo PAT.

`ci.py pkg` checks the result with `pkgutil --check-signature`; anything
other than "signed by a developer certificate issued by Apple …
notarization: ok" fails the build before any artifact is uploaded.

## CI signing

The Developer ID secrets are bound on the **private** repo (the build
runs there). Same env-var names as the table above plus the base64 `.p12`
imports the runner keychain needs — 9 secrets total:

- `APPLE_SIGNING_IDENTITY`, `APPLE_CERTIFICATE`,
  `APPLE_CERTIFICATE_PASSWORD` — Developer ID **Application** cert (signs
  the `.app` + CLI under the hardened runtime).
- `APPLE_INSTALLER_SIGNING_IDENTITY`, `APPLE_INSTALLER_CERTIFICATE`,
  `APPLE_INSTALLER_CERTIFICATE_PASSWORD` — Developer ID **Installer**
  cert (signs the outer `.pkg`).
- `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` — notarization
  (`notarytool`).

These are the same values this repo used pre-F31.5; F31.5 copied them to
the private repo. See [RELEASE.md](../RELEASE.md) for the cross-repo
release pipeline.

## Troubleshooting

- **"User interaction is not allowed"** from `codesign` on CI → the
  keychain needs to be unlocked: `security unlock-keychain -p "$PASS"
  login.keychain`.
- **Notarization rejected** with hardened-runtime violations → inspect
  `xcrun notarytool log <submission-id>`; most fixes are entitlements
  adjustments in `Entitlements.plist`.
- **Stapling fails** with "could not find a ticket" → notarization is
  still running; `notarytool submit --wait` should block until done, but
  double-check status with `notarytool info <submission-id>`.
