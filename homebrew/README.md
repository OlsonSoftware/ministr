# Homebrew Tap Setup

## One-time setup

1. Create a new GitHub repository: `AlrikOlson/homebrew-tap`
2. Copy `ministr.rb` to `Formula/ministr.rb` in that repository
3. Push to GitHub

Users can then install with:

```sh
brew install AlrikOlson/tap/ministr
```

## Updating after a release

After pushing a new `v*` tag and the release workflow completes:

1. Download the `.sha256` files from the GitHub Release assets
2. Update the `version` and `sha256` values in `Formula/ministr.rb`
3. Push to the `homebrew-tap` repository

### Automation

Add a step to the release workflow that auto-updates the tap:

```yaml
- name: Update homebrew tap
  uses: mislav/bump-homebrew-formula-action@v3
  with:
    formula-name: ministr
    homebrew-tap: AlrikOlson/homebrew-tap
  env:
    COMMITTER_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
```

Requires a personal access token with `repo` scope stored as `HOMEBREW_TAP_TOKEN`.
