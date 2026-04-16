# Homebrew Tap Setup

## One-time setup

1. Create a new GitHub repository: `AlrikOlson/homebrew-iris`
2. Copy `iris.rb` to `Formula/iris.rb` in that repository
3. Push to GitHub

Users can then install with:

```sh
brew install AlrikOlson/tap/iris
```

## Updating after a release

After pushing a new `v*` tag and the release workflow completes:

1. Download the `.sha256` files from the GitHub Release assets
2. Update the `version` and `sha256` values in `Formula/iris.rb`
3. Push to the `homebrew-iris` repository

### Automation (optional)

Add a step to the release workflow that auto-updates the tap:

```yaml
- name: Update homebrew tap
  uses: mislav/bump-homebrew-formula-action@v3
  with:
    formula-name: iris
    homebrew-tap: AlrikOlson/homebrew-iris
  env:
    COMMITTER_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
```

This requires a personal access token with `repo` scope stored as `HOMEBREW_TAP_TOKEN`.
