# CI Runner Setup — OlsonSoftware

One-time setup so this repo's CI stops OOMing on stock GitHub runners. Do
this **before** the optimized workflows can use the big runner; until you
do, every job falls back to free `ubuntu-latest` (correct, just slow / may
OOM on the 3 heavy jobs).

You configure up to three things:

1. **A large Linux runner** the heavy CI jobs target — **Path A** (self-hosted) or **Path B** (GitHub-hosted larger).
2. **A Cloudflare R2 bucket** for the shared `sccache` compile cache (S1).
3. *(Optional, recommended)* **A self-hosted Windows runner on your own machine** — **Path C** — for the slow tagged-release Windows builds.

Then you set the Actions **variables + secrets** in S2 so the workflows
pick it all up. No YAML edits on your side — the workflows read
`vars.CI_RUNNER`, the optional `vars.CI_RUNNER_WINDOWS`, and the
`SCCACHE_*` secrets. Every input is independent and optional: an unset
variable just falls back to a GitHub-hosted runner, so partial setup
never breaks CI.

> Why: only 3 CI jobs (`rust-dev`, `rust-release`, `docker-build`) compile
> the Rust workspace; they're path-gated so they only run on Rust-source
> pushes. Those need ~4 GB RAM/vCPU + lots of NVMe. Everything else stays
> on free `ubuntu-latest`. sccache makes even the heavy jobs finish in
> ~3–5 min instead of ~15.

---

## Decision: which runner path?

| | **A. Self-hosted (Hetzner)** — recommended | **B. GitHub-hosted larger** |
|---|---|---|
| Cost | ~$77/mo flat (CCX43, any volume) | ~$0.04/min, only while running (~$1–1.5 per Rust push) |
| Ops | You patch the box | Zero |
| Speed | Fastest (warm local cache) | Fast (sccache/R2) |
| Best when | You push Rust often | Infrequent Rust pushes / no-ops preference |

Both use the **same** Actions variable, so you can switch later by changing one value.

---

## Path A — Self-hosted runner (Hetzner CCX43)

### A1. Provision the box
- Provider: **Hetzner Cloud** → **CCX43** (16 dedicated vCPU / 64 GB RAM / 360 GB NVMe), Ubuntu **24.04** x86_64. (CCX33 = 8 vCPU/32 GB is the minimum acceptable.)
- After first boot, SSH in and:

```bash
# 32 GB swap — OOM safety net for candle/LLVM/esaxx spikes
sudo fallocate -l 32G /swapfile && sudo chmod 600 /swapfile
sudo mkswap /swapfile && sudo swapon /swapfile
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab

# Docker (CI jobs run inside the ministr-ci container)
curl -fsSL https://get.docker.com | sudo sh

# Dedicated unprivileged user for the runner
sudo useradd -m -s /bin/bash gha && sudo usermod -aG docker gha
```

### A2. Create an org runner group (scoped to this repo only)
1. GitHub → **OlsonSoftware org** → **Settings** → **Actions** → **Runner groups** → **New runner group**.
2. Name: `ministr-rust`.
3. **Repository access**: *Selected repositories* → add **OlsonSoftware/ministr** only.
4. Save.

### A3. Register **two** runner agents on the one box
Two agents = `rust-dev` and `rust-release` (which run in parallel) share
the box and its warm cache instead of serializing.

1. GitHub → **OlsonSoftware org** → Settings → Actions → Runners → **New runner** → **New self-hosted runner** → Linux x64. **Must be the _org_ page** — runner groups are an org-level feature, so the `--url` is `https://github.com/OlsonSoftware` (the org, *not* `.../ministr`) and the token is an org token. Copy the exact `--url`/`--token` shown there.
2. On the box, as the `gha` user, install **two** runner instances:

```bash
sudo -iu gha
for i in 1 2; do
  mkdir -p ~/actions-runner-$i && cd ~/actions-runner-$i
  curl -o r.tar.gz -L https://github.com/actions/runner/releases/latest/download/actions-runner-linux-x64.tar.gz
  tar xzf r.tar.gz
  ./config.sh \
    --url https://github.com/OlsonSoftware \
    --token <ORG_REGISTRATION_TOKEN> \
    --runnergroup ministr-rust \
    --labels ministr-rust \
    --name "ministr-rust-$i" \
    --work _work --unattended --replace
  sudo ./svc.sh install gha && sudo ./svc.sh start
  cd ~
done
```

> The runner registers at the **org** and is restricted to this repo by
> the `ministr-rust` group's *Repository access → OlsonSoftware/ministr*
> setting (S/A2) — not by the URL. Using the repo URL `.../ministr` with
> `--runnergroup` returns **404 Not Found** at registration. Get a fresh
> `<ORG_REGISTRATION_TOKEN>` from the org **New runner** page for each
> (single-use, ~1 h TTL).

### A4. Disk hygiene (cron)
The debug `--all-targets` tree is large. Add a weekly prune as `gha`:

```bash
( crontab -l 2>/dev/null; echo '0 5 * * 0 docker system prune -af --filter "until=168h" >/dev/null 2>&1' ) | crontab -
```

→ Continue to **Shared steps** below.

---

## Path B — GitHub-hosted larger runner

1. GitHub → **OlsonSoftware org** → **Settings** → **Actions** → **Runners** → **New runner** → **New GitHub-hosted runner**.
2. Name: `ministr-rust`.
3. Platform: **Linux**, Image: **Ubuntu 24.04**.
4. Size: **16-vcpu (64 GB RAM, 600 GB disk)** — or 8-vcpu (32 GB) to cut cost.
5. **Auto-scaling**: Max concurrency ≥ **2** (so `rust-dev` + `rust-release` run in parallel).
6. **Runner group**: create/select `ministr-rust`, **Repository access → Selected → OlsonSoftware/ministr**.
7. Save. The runner's label is its name: `ministr-rust`.

→ Continue to **Shared steps** below.

---

## Path C — Self-hosted Windows runner (your local machine)

`release.yml`'s **Windows** shards are **by far the slowest release
build**: the CLI `.zip` plus the Tauri desktop `.exe` (CLI sidecar
compiled with `--features directml`, then NSIS bundling) — all while
Defender real-time-scans every `.rmeta`/`.obj`. They only run on `v*`
tags, so the runner is **idle ~all the time** and only works during a
release. Running it on your own Windows 11 box (the one this repo lives
on) is the single biggest release-time win after the Linux runner, at
zero recurring cost.

> **Scope:** this runner serves **only** the two Windows shards in
> `release.yml` (`cli` → `x86_64-pc-windows-msvc`, `desktop` →
> `windows-x86_64`). It does **not** run `ci.yml` — those jobs execute
> inside a Linux `container:` and require a Linux runner. So nothing
> here affects day-to-day PR CI; it just makes tagged releases fast.

### C0. (Recommended) Isolate the runner

A self-hosted runner executes whatever the workflow + every build
script + every transitive crate's `build.rs` does, with your user's
privileges. The repo is private (org-only authors), so the realistic
threat is **supply-chain / build-script** rather than fork PRs — but
you still don't want that touching your dev box directly. Windows 11
**Pro** (what you run) gives you two native, no-cost sandboxes:

| | **Hyper-V VM** — recommended | **Windows Sandbox** |
|---|---|---|
| Persistence | Yes (warm toolchain, runs as a boot service) | None — wiped on close |
| Isolation | Full VM (separate kernel/disk/network) | Full VM, disposable |
| Best for | A runner that's online for releases | One-off "spin up just before tagging" |
| Cost | $0 | $0 |

**Recommended: a dedicated Hyper-V VM.** It's a real, isolated machine
that keeps the toolchain warm and runs the runner service at boot, with
none of the build touching your host:

1. Enable Hyper-V (elevated PowerShell, one-time, reboots):
   ```powershell
   Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V -All
   ```
2. Hyper-V Manager → **Quick Create** → *Windows 11 dev environment*
   (or your own Win11 ISO). Give it **8 vCPU / 16 GB / 120 GB** dynamic
   disk, Default Switch (NAT networking is enough — the runner only
   makes outbound HTTPS to GitHub).
3. **Snapshot** ("checkpoint") the clean VM before installing anything,
   so you can revert after a bad dependency.
4. Do **C1–C4 below inside the VM**, not on your host. The host stays
   pristine; the only thing crossing the boundary is the runner's
   outbound connection to GitHub.

**Alternative: Windows Sandbox** (fully disposable, zero residue) — good
if you'd rather have *nothing* persist and just launch it before a
release. Caveats tailored to our build: it's **ephemeral**, so every
release is a cold build (no cargo/sccache cache survives — acceptable
since Windows releases are infrequent), and you must auto-provision it
each launch via a `.wsb` config with a `<LogonCommand>` that installs
the prereqs + registers an `--ephemeral` runner with a fresh token.
Enable once with:
```powershell
Enable-WindowsOptionalFeature -Online -FeatureName "Containers-DisposableClientVM" -All
```
Then C1–C4 run inside the Sandbox session (add `--ephemeral` to the
`config.cmd` in C4 so the runner deregisters after one job and the
next launch starts clean).

> No GPU is needed in either sandbox: `--features directml` only
> *compiles* the DirectML bindings; nothing in the Windows release
> shards runs GPU inference. Plain VM/Sandbox CPU is fine.

If you accept the risk and skip isolation, run C1–C4 directly on the
host — the Defender exclusions in C2 are then scoped as tightly as
possible, but the build still runs as your user.

### C1. Prerequisites (install once, in order)

> If you chose C0's Hyper-V VM or Windows Sandbox, run **C1–C4 inside
> that sandbox**, not on the host.

Run an **elevated PowerShell** (`Win+X` → *Terminal (Admin)*). These
match exactly what the `release.yml` Windows steps assume a runner has —
GitHub's hosted `windows-latest` ships them; your box must too.

> **Fresh Windows / Sandbox / VM gotcha:** the Microsoft Store
> (`msstore`) winget source often fails first run with
> `0x8a15005e : The server certificate did not match…`. Harmless here —
> everything we need is on the community `winget` source. The commands
> below all pass `--source winget` to skip `msstore` entirely. (To
> silence it globally instead: `winget source reset --force`.)

```powershell
# winget is built into Windows 11 Pro. --source winget avoids the
# msstore cert error on fresh installs; accept agreements once.
winget install --id Git.Git              --source winget -e --accept-source-agreements --accept-package-agreements
winget install --id Microsoft.PowerShell --source winget -e   # `pwsh` 7 — release.yml has `shell: pwsh` steps
winget install --id Kitware.CMake        --source winget -e   # ort-sys / tree-sitter build scripts
winget install --id Rustlang.Rustup      --source winget -e
# C/C++ toolchain for the native deps (ort-sys, tokenizers/esaxx C++,
# tree-sitter C, libsqlite3-sys). The C++ workload includes MSVC + the
# Windows 11 SDK.
winget install --id Microsoft.VisualStudio.2022.BuildTools --source winget -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Then, in a **new** terminal so PATH refreshes:

```powershell
rustup default stable
rustup target add x86_64-pc-windows-msvc      # the only target these shards build
rustc --version ; cmake --version ; bash --version ; pwsh --version
```

Why each matters (don't skip — each maps to a real `release.yml` step):

- **Git for Windows** → provides `bash`. `release.yml`'s "Build release
  binary" and "Use lld linker" steps are `shell: bash`; without Git
  Bash on PATH they fail outright.
- **PowerShell 7 (`pwsh`)** → the "Disable Defender" and "Package
  (Windows)" steps are `shell: pwsh` (not Windows PowerShell 5.1, which
  is all Win11 ships by default).
- **VS 2022 Build Tools (VCTools)** → MSVC `cl.exe` + Windows SDK for
  the native C/C++ crates. (Linking itself uses bundled `rust-lld`,
  which the workflow configures and which ships with the Rust
  toolchain — no extra install.)
- **CMake** → `ort-sys` and several `tree-sitter-*` build scripts.
- **WebView2 Runtime** → required by the Tauri desktop bundle. Windows
  11 ships the Evergreen runtime preinstalled; verify with:
  ```powershell
  Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -EA SilentlyContinue
  ```
  If that returns nothing, install **Microsoft Edge WebView2 Runtime**
  (Evergreen Standalone) from Microsoft.
- **Node / pnpm** → *not* preinstalled here on purpose: the desktop
  shard's `actions/setup-node@v4` (Node 22) and `pnpm/action-setup@v4`
  (pnpm 10) provision them per-run and work fine on a self-hosted
  runner. Leave it to the workflow.

### C2. Permanent Defender exclusions

`release.yml` adds these per-run, but setting them **permanently** on
the host removes the scan tax from the *entire* build, not just after
the exclusion step runs. Elevated PowerShell:

```powershell
$work = "C:\actions-runner\_work"
New-Item -ItemType Directory -Force -Path $work | Out-Null
Add-MpPreference -ExclusionPath $work
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.cargo"
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.rustup"
foreach ($p in 'rustc.exe','cargo.exe','link.exe','lld-link.exe','cl.exe','sccache.exe') {
  Add-MpPreference -ExclusionProcess $p
}
```

> This is your dev machine — these exclusions are scoped to the runner
> work dir + the Rust toolchain dirs + compiler processes, not
> system-wide. Keep ≥ 30 GB free where the runner lives (release
> `target/` for ORT/tokenizers/tauri is ~10 GB).

### C3. Create the org runner group (scoped to this repo)

GitHub → **OlsonSoftware org** → **Settings** → **Actions** → **Runner
groups**. Reuse **`ministr-rust`** if you already made it for the Linux
runner (it's already scoped to `OlsonSoftware/ministr`); otherwise
create it now with *Repository access → Selected → OlsonSoftware/ministr*.

### C4. Register the runner as a Windows service

GitHub → **OlsonSoftware org** → Settings → Actions → Runners → **New
runner** → **New self-hosted runner** → **Windows / x64**. This **must
be the org-level page** (org runner groups don't exist at repo scope) —
it shows `--url https://github.com/OlsonSoftware` (the org, *not*
`.../ministr`) and an org registration token. Copy that token, then in
an **elevated PowerShell**:

```powershell
mkdir C:\actions-runner; cd C:\actions-runner
$ver = (Invoke-RestMethod https://api.github.com/repos/actions/runner/releases/latest).tag_name.TrimStart('v')
Invoke-WebRequest -Uri "https://github.com/actions/runner/releases/download/v$ver/actions-runner-win-x64-$ver.zip" -OutFile runner.zip
Expand-Archive -Path runner.zip -DestinationPath . -Force; Remove-Item runner.zip

.\config.cmd `
  --url https://github.com/OlsonSoftware `
  --token <ORG_REGISTRATION_TOKEN> `
  --runnergroup ministr-rust `
  --labels ministr-windows `
  --name "ministr-win-$env:COMPUTERNAME" `
  --work _work `
  --runasservice `
  --unattended --replace
```

> **404 at "Authentication" / registration** = wrong scope: the URL was
> the repo (`.../ministr`) or the token was a repo token while
> `--runnergroup` (an org concept) was set. Use the **org** URL
> `https://github.com/OlsonSoftware` + an **org** token from the org
> New-runner page. The runner is still restricted to this repo — that's
> enforced by the `ministr-rust` group's *Repository access* setting
> (S2 / step C3), not the URL.

- `--runasservice` installs it as the `actions.runner.*` Windows
  service → it auto-starts at boot and is online whenever your machine
  is, with no console window. Manage it later via `services.msc` or
  `.\svc.cmd status|stop|start`.
- The **label `ministr-windows`** is the only thing the workflow keys
  off (via `vars.CI_RUNNER_WINDOWS`).
- Get a fresh `<REGISTRATION_TOKEN>` from the *New runner* page each
  time (single-use, ~1 h TTL).

> Optional — parallelism: a tagged release runs the **cli** and
> **desktop** Windows shards concurrently. One runner service runs them
> one-after-the-other (fine; releases are rare). For true parallel,
> repeat C4 into `C:\actions-runner-2` with `--name ministr-win-2`
> (same label/group).

→ Continue to **Shared steps** below (S2 is where you set
`CI_RUNNER_WINDOWS = ministr-windows`).

---

## Shared steps (both paths)

### S1. Create the Cloudflare R2 bucket (sccache backend)
1. Cloudflare dashboard → **R2** → **Create bucket** → name `ministr-sccache` (any region; R2 has no egress fees).
2. R2 → **Manage R2 API Tokens** → **Create API token**:
   - Permissions: **Object Read & Write**
   - Scope: the `ministr-sccache` bucket only
   - Create → copy the **Access Key ID** and **Secret Access Key**.
3. Note your account's S3 endpoint: `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` (R2 → bucket → Settings → S3 API).

### S2. Set the Actions variable + secrets
On **OlsonSoftware/ministr** → **Settings** → **Secrets and variables** → **Actions**:

**Variables** tab → **New repository variable**:

| Name | Value | Notes |
|---|---|---|
| `CI_RUNNER` | `ministr-rust` | Linux heavy jobs + Linux release shards |
| `CI_RUNNER_WINDOWS` | `ministr-windows` | *Optional* — only if you did Path C. Routes the slow Windows release shards. Unset → stays on hosted `windows-latest`. |

**Secrets** tab → **New repository secret** (×4):

| Name | Value |
|---|---|
| `SCCACHE_BUCKET` | `ministr-sccache` |
| `SCCACHE_ENDPOINT` | `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` |
| `SCCACHE_R2_ACCESS_KEY_ID` | R2 token Access Key ID |
| `SCCACHE_R2_SECRET_ACCESS_KEY` | R2 token Secret Access Key |

> Every input is independently optional. `vars.CI_RUNNER` falls back to `ubuntu-latest`, `vars.CI_RUNNER_WINDOWS` to hosted `windows-latest`, and the `SCCACHE_*` secrets to a local cache. A partial setup degrades gracefully — it never breaks CI; it's just slower until complete.

### S3. (If you use branch protection) required checks
The single required status check is **`ci complete`** (job `ci-complete`).
No change needed — it stays green when path-gated jobs are skipped. Just
confirm it's still the required check after these workflow updates land.

### S4. Verify

**Linux runner / sccache:**
1. Push a trivial Rust change (e.g. a comment in `ministr-core/src/lib.rs`) on a branch → open a PR.
2. Actions tab: `rust-dev` / `rust-release` run **on `ministr-rust`** (check the runner name in the job log header); `fmt` / `security` / `changes` on `ubuntu-latest`.
3. First run is a cold sccache. A second push on the same branch should show `sccache` hits in the `rust-dev` log and finish in ~3–5 min.
4. Docs-only / markdown pushes: confirm `rust-*` are **skipped** (no big-runner spend).

**Windows runner (Path C):**
5. Confirm the runner is **Idle** (green) at org → Settings → Actions → Runners — it stays idle until a tag.
6. Cut a prerelease tag to exercise it without shipping: `git tag v0.0.0-rc.test && git push origin v0.0.0-rc.test`. In the **Release** workflow run, the `CLI x86_64-pc-windows-msvc` and `Desktop windows-x86_64` jobs should show your machine's runner name (`ministr-win-…`); the other shards stay on hosted macOS/Linux. Delete the test tag/release afterwards (`git push origin :v0.0.0-rc.test`).

---

## Cost summary

| Setup | Monthly | Per Rust push |
|---|---|---|
| A: Hetzner CCX43 self-hosted | ~$77 flat + ~$7 platform fee | $0 marginal |
| A: Hetzner CCX33 (8/32) | ~$40 flat | $0 marginal |
| B: GitHub 16-vcpu larger | $0 idle | ~$0.30–0.60 (sccache-warm) |
| C: Windows on your own box | **$0** (electricity) | $0 — only runs on `v*` tags |
| Non-Rust / docs pushes | — | $0 (path-gated) |
| R2 sccache storage | ~$0 (a few GB, no egress) | — |

Once `CI_RUNNER` / `CI_RUNNER_WINDOWS` are set (and the secrets exist),
the optimized workflows — already committed on the
`feat/unified-installer-experience` branch — use all of this
automatically. Every input is optional and degrades gracefully: an
unset variable just falls back to the GitHub-hosted runner.
