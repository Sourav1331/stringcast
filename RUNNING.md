# Running Stringcast (Developer / Local)

Quick guide to build, configure, and run Stringcast locally.

## Prerequisites
- Rust toolchain (rustup + cargo).
- Platform native dependencies for input simulation and X11 (Linux example):

  Debian/Ubuntu:
  ```bash
  sudo apt update
  sudo apt install build-essential libxdo-dev libx11-dev libxi-dev libxtst-dev
  ```

  Fedora:
  ```bash
  sudo dnf install @development-tools libxdo-devel libX11-devel libXi-devel libXtst-devel
  ```

  Arch:
  ```bash
  sudo pacman -S base-devel xdotool libx11 libxi libxtst
  ```

- On macOS/Windows: ensure accessibility/input-monitoring permissions are available to the app when running.

## 1) Build

Debug build:

```bash
cargo build
```

Release build:

```bash
cargo build --release
```

## 2) Initialize config
Create a default config file (if none exists):

```bash
# Using the built binary
./target/debug/stringcast init
# or with cargo
cargo run -- init
```

Show config path:

```bash
./target/debug/stringcast show-config
# or
cargo run -- show-config
```

## 3) Add an API key
Set your provider secret in the environment and add key metadata:

```bash
export STRINGCAST_API_KEY="your-secret-here"
./target/debug/stringcast add-key openai key-1 "Main"
# or
STRINGCAST_API_KEY="your-secret-here" cargo run -- add-key openai key-1 "Main"
```

Supported providers:
- `gemini`
- `openai`
- `anthropic`
- `custom` for any OpenAI-compatible endpoint

Examples:

```bash
STRINGCAST_API_KEY="your-secret-here" cargo run -- add-key gemini key-1 "Gemini"
STRINGCAST_API_KEY="your-secret-here" cargo run -- add-key anthropic key-1 "Claude"
STRINGCAST_API_KEY="your-secret-here" cargo run -- add-key custom key-1 "Custom API"
```

Use the provider that matches the API key you are storing, then select it as the active provider in the next step.

## 4) Select provider

Choose the active provider with one of these commands:

```bash
./target/debug/stringcast set-provider gemini
./target/debug/stringcast set-provider openai
./target/debug/stringcast set-provider anthropic
./target/debug/stringcast set-provider custom
```

If you prefer `cargo run`:

```bash
cargo run -- set-provider gemini
cargo run -- set-provider openai
cargo run -- set-provider anthropic
cargo run -- set-provider custom
```

Use the same provider name here that you used when adding the API key.

## 5) Enable and run

Enable the service in config (persists):

```bash
./target/debug/stringcast enable
```

Run the runtime (foreground; logs printed to terminal):

```bash
./target/debug/stringcast run
# or
cargo run -- run
```

To run the release binary:

```bash
./target/release/stringcast run
```

Ctrl+C stops the process.

## 6) Useful commands

Check permissions (macOS/Windows):

```bash
./target/debug/stringcast check-permissions
```

View status (enabled, provider, keys):

```bash
./target/debug/stringcast status
```

Run tests:

```bash
cargo test
```

## 7) Running as a background service (example: systemd)
Create `/etc/systemd/system/stringcast.service` (example):

```ini
[Unit]
Description=Stringcast background service
After=network.target

[Service]
Type=simple
User=YOUR_USER
WorkingDirectory=/home/YOUR_USER/Documents/stringcast
ExecStart=/home/YOUR_USER/Documents/stringcast/target/release/stringcast run
Restart=on-failure

[Install]
WantedBy=default.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now stringcast.service
```

On macOS you'd use `launchd` or a user agent; on Windows use a scheduled task or NSSM/svc wrapper.

## 8) Linux Wayland fallback

The primary Stringcast runtime is the Rust app. A separate Python Wayland listener exists as an experimental Linux-only fallback for environments where normal global keyboard hooks are blocked.

See [docs/WAYLAND_POC.md](docs/WAYLAND_POC.md) before using it. It reads keyboard devices from `/dev/input`, injects events through `uinput`, and therefore has sensitive permissions and security tradeoffs.

## 9) Troubleshooting
- Linker errors on Linux often mean missing native dev packages (e.g., `-lxdo`). Install `libxdo-dev` / `xdotool` system packages.
- If replacements re-trigger the app, verify `SyntheticInputGuard` behavior and ensure the app's simulated input is suppressed by the listener.
- If clipboard operations fail in certain apps, those apps may block select/copy; see `SPEC.md` for fallback behavior.

## 10) Notes
- This repo uses select-all + clipboard to read active-field text. The app restores the clipboard after each operation.
- For development, `cargo run -- run` is easiest; for production, prefer `cargo build --release` and run the release binary.
- For Windows-specific setup, installation, usage, and troubleshooting, see [docs/WINDOWS.md](docs/WINDOWS.md).

---
For architecture details and behavioral specs, see [SPEC.md](SPEC.md).
