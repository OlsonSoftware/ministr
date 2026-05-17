# CI Runner Setup — OlsonSoftware

One-time setup so this repo's CI stops OOMing on stock GitHub runners. Do
this **before** the optimized workflows can use the big runner; until you
do, every job falls back to free `ubuntu-latest` (correct, just slow / may
OOM on the 3 heavy jobs).

You configure two things:

1. **A large runner** the heavy jobs target (self-hosted *or* GitHub-hosted larger).
2. **A Cloudflare R2 bucket** for the shared `sccache` compile cache.

Then you set **one Actions variable + four secrets** so the workflows pick it all up. No YAML edits on your side — the workflows read `vars.CI_RUNNER` and the `SCCACHE_*` secrets.

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

1. GitHub → org → Settings → Actions → Runners → **New runner** → **New self-hosted runner** → Linux x64. Copy the `./config.sh` URL + token.
2. On the box, as the `gha` user, install **two** runner instances:

```bash
sudo -iu gha
for i in 1 2; do
  mkdir -p ~/actions-runner-$i && cd ~/actions-runner-$i
  curl -o r.tar.gz -L https://github.com/actions/runner/releases/latest/download/actions-runner-linux-x64.tar.gz
  tar xzf r.tar.gz
  ./config.sh \
    --url https://github.com/OlsonSoftware/ministr \
    --token <REGISTRATION_TOKEN> \
    --runnergroup ministr-rust \
    --labels ministr-rust \
    --name "ministr-rust-$i" \
    --work _work --unattended --replace
  sudo ./svc.sh install gha && sudo ./svc.sh start
  cd ~
done
```

> Get a fresh `<REGISTRATION_TOKEN>` for each from the **New runner** page (they're single-use, ~1 h TTL). The critical part is `--labels ministr-rust`.

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

| Name | Value |
|---|---|
| `CI_RUNNER` | `ministr-rust` |

**Secrets** tab → **New repository secret** (×4):

| Name | Value |
|---|---|
| `SCCACHE_BUCKET` | `ministr-sccache` |
| `SCCACHE_ENDPOINT` | `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` |
| `SCCACHE_R2_ACCESS_KEY_ID` | R2 token Access Key ID |
| `SCCACHE_R2_SECRET_ACCESS_KEY` | R2 token Secret Access Key |

> The workflows read `vars.CI_RUNNER` for `runs-on` (fallback `ubuntu-latest` if unset) and the `SCCACHE_*` secrets for the compile cache (sccache silently falls back to a local cache if they're absent). So a partial setup degrades gracefully — it never breaks CI.

### S3. (If you use branch protection) required checks
The single required status check is **`ci complete`** (job `ci-complete`).
No change needed — it stays green when path-gated jobs are skipped. Just
confirm it's still the required check after these workflow updates land.

### S4. Verify
1. Push a trivial Rust change (e.g. a comment in `ministr-core/src/lib.rs`) on a branch → open a PR.
2. Actions tab: `rust-dev` / `rust-release` should run **on `ministr-rust`** (check the runner name in the job log header), `fmt` / `security` / `changes` on `ubuntu-latest`.
3. First run is a cold sccache (slow-ish). Second push on the same branch should show `sccache` cache hits in the `rust-dev` log and finish in ~3–5 min.
4. Docs-only or markdown pushes: confirm `rust-*` are **skipped** (no big-runner spend).

---

## Cost summary

| Setup | Monthly | Per Rust push |
|---|---|---|
| A: Hetzner CCX43 self-hosted | ~$77 flat + ~$7 platform fee | $0 marginal |
| A: Hetzner CCX33 (8/32) | ~$40 flat | $0 marginal |
| B: GitHub 16-vcpu larger | $0 idle | ~$0.30–0.60 (sccache-warm) |
| Non-Rust / docs pushes | — | $0 (path-gated) |
| R2 sccache storage | ~$0 (a few GB, no egress) | — |

Once `CI_RUNNER` is set and the secrets exist, the optimized workflows
(already committed on the `feat/unified-installer-experience` branch) use
all of this automatically.
