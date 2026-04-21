# macOS code signing & notarization

The iris desktop app and the bundled `iris` CLI both need to be signed
with a **Developer ID Application** certificate and notarized by Apple
before they can be distributed to end users. Without signing, Gatekeeper
will block launch on anything other than the developer's own machine.

Out of the box this repo builds **unsigned**. Signing is off until an
identity is configured — `iris-app/src-tauri/tauri.conf.json` has
`bundle.macOS.signingIdentity: null`, and `just pkg-dev` explicitly
skips notarization.

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

`scripts/build-pkg.sh` (invoked by `just pkg`) reads:

| Variable                       | Example                                             | Required by                |
| ------------------------------ | --------------------------------------------------- | -------------------------- |
| `APPLE_SIGNING_IDENTITY`       | `Developer ID Application: Your Name (TEAMID)`      | signing (always)           |
| `APPLE_INSTALLER_IDENTITY`     | `Developer ID Installer: Your Name (TEAMID)`        | `.pkg` signing (always)    |
| `APPLE_ID`                     | `you@example.com`                                   | notarization               |
| `APPLE_PASSWORD`               | app-specific password                               | notarization               |
| `APPLE_TEAM_ID`                | `ABCD123456`                                        | notarization               |
| `SKIP_NOTARIZE`                | `1`                                                 | dev-only bypass            |

Store these in `.env.signing` at the repo root (already gitignored), then
`source .env.signing` before running `just pkg`.

## Wire up Tauri's bundler

In `iris-app/src-tauri/tauri.conf.json`, set:

```json
"macOS": {
  "signingIdentity": "Developer ID Application: Your Name (TEAMID)",
  "entitlements": "./Entitlements.plist",
  "minimumSystemVersion": "13.0"
}
```

The entitlements file at `iris-app/src-tauri/Entitlements.plist` already
grants the three entitlements WKWebView needs under hardened runtime
(`allow-jit`, `allow-unsigned-executable-memory`,
`allow-dyld-environment-variables`). Do not add entitlements beyond what
the app actually uses — Apple scrutinizes broad entitlements.

`signingIdentity` in `tauri.conf.json` is only consulted by the
`.dmg`/`.app` bundler path (`pnpm tauri build`). The `.pkg` path in
`scripts/build-pkg.sh` reads the same identity from the env var directly
via `codesign` and `productbuild`.

## Building

Local signed build, skipping notarization (fast, for checking that
signing is wired up right):

```
just pkg-dev
```

Signed + notarized + stapled `.pkg` for distribution:

```
source .env.signing
just pkg
```

The final `.pkg` lands in `target/pkg/iris-<version>.pkg`. `pkgutil
--check-signature` runs automatically; if it says anything other than
"signed by a developer certificate issued by Apple … notarization: ok"
the release is not ready to ship.

## CI signing

`.github/workflows/app-release.yml` triggers on `v*-app` tags and calls
`tauri-apps/tauri-action` which respects the same env vars. Add them as
repo secrets:

- `APPLE_SIGNING_IDENTITY`
- `APPLE_ID`
- `APPLE_PASSWORD`
- `APPLE_TEAM_ID`
- `APPLE_CERTIFICATE` — base64-encoded `.p12` of the Developer ID
  Application cert (required by tauri-action to import into the runner
  keychain)
- `APPLE_CERTIFICATE_PASSWORD` — password for the `.p12` export

The `.pkg` workflow (not yet in CI — currently `just pkg` is a local
recipe) would need `APPLE_INSTALLER_IDENTITY` plus the corresponding
`.p12` import for the Installer cert.

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
