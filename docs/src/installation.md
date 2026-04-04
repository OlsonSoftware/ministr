# Installation

## CLI (all platforms)

### From source

```sh
cargo install --path iris-cli
```

### From GitHub Releases

Download the binary for your platform from the [Releases page](https://github.com/alrik/iris-rs/releases).

## Desktop App

### macOS

1. Download `iris_x.x.x_aarch64.dmg` (Apple Silicon) or `iris_x.x.x_x64.dmg` (Intel)
2. Open the DMG and drag iris to Applications
3. Launch iris from Applications — it appears as a system tray icon
4. Right-click the tray icon to add projects

### Windows

1. Download `iris_x.x.x_x64-setup.exe`
2. Run the installer
3. iris starts automatically and appears in the system tray
4. Right-click the tray icon to manage projects

### Linux

#### AppImage

```sh
chmod +x iris_x.x.x_amd64.AppImage
./iris_x.x.x_amd64.AppImage
```

#### Debian/Ubuntu

```sh
sudo dpkg -i iris_x.x.x_amd64.deb
```

## Auto-start

The desktop app can be configured to start at login:

- **Settings > Startup > Start at login** toggle in the GUI
- Or manually:
  - **macOS**: Copy `com.iris.app.plist` to `~/Library/LaunchAgents/`
  - **Windows**: Managed via registry by the autostart plugin
  - **Linux**: XDG autostart entry at `~/.config/autostart/iris.desktop`

## Configuration

Global config file: `~/.iris/config.toml`

```toml
default_model = "nomic-embed-text-v1.5"

[data]
dir = "~/.iris"
```

Per-project config: `.iris.toml` in the project root

```toml
[[corpus]]
paths = ["src/", "docs/"]
ignore = ["target/", "node_modules/"]

[[corpus.cloud]]
url = "https://example.com/project.iris-index"
```
