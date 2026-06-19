# Stringcast

Stringcast is a system-wide AI text transformation tool. Type text in a field, end it with a trigger such as `?fix`, and Stringcast sends the selected text to your configured AI provider, shows a short working marker, then replaces the field with the result.

The current app is a Rust CLI/runtime MVP. It works from a terminal today; packaged desktop apps and installers are planned next.

## Status

- Primary runtime: Rust.
- macOS: actively tested during development.
- Windows: runtime scaffolding exists, needs real user testing.
- Linux X11: intended to use the Rust runtime.
- Linux Wayland: separate experimental Python fallback exists in [docs/WAYLAND_POC.md](docs/WAYLAND_POC.md).

For macOS and Windows users, the preferred direction is downloadable Rust binaries/apps, not a separate Python implementation.

## Features

- System-wide trigger detection.
- Clipboard-based text extraction and replacement.
- Inline working marker while the API request is in flight.
- API key metadata in config and secrets in OS key storage.
- Provider support for Gemini, OpenAI, Anthropic, and custom OpenAI-compatible APIs.
- Built-in commands for grammar, tone, summary, translation, and more.

## Commands

Static commands:

```text
?fix        Fix grammar, spelling, and punctuation
?improve    Improve clarity and readability
?shorten    Shorten text
?expand     Expand with more detail
?formal     Rewrite formally
?casual     Rewrite casually
?emoji      Add tasteful emojis
?reply      Generate a reply
?bullets    Convert to bullet points
?summarize  Summarize in 1-3 sentences
```

Dynamic commands:

```text
?translate:<lang>
?ask:<question>
```

Examples:

```text
i dont knwo whats happening ?fix
hello, how are you ?translate:hi
This paragraph is too long. ?ask:make it sound more confident
```

## Quick Start From Source

Install Rust, then run:

```bash
cargo run -- init
cargo run -- check-permissions
```

Add an API key:

```bash
STRINGCAST_API_KEY="your-key-here" cargo run -- add-key gemini main "Gemini"
cargo run -- set-provider gemini
cargo run -- api-test
```

Run Stringcast:

```bash
cargo run -- run
```

Type in a normal text field:

```text
i dont knwo whats happening ?fix
```

For detailed local setup, see [RUNNING.md](RUNNING.md).

## Downloadable Builds

Release workflows build downloadable binaries for:

- macOS
- Windows
- Linux

Download artifacts from the GitHub Actions release workflow or from GitHub Releases once release publishing is enabled.

See [docs/RELEASES.md](docs/RELEASES.md) for artifact download and smoke-test steps.

The macOS release also includes an unsigned `Stringcast.app` bundle with a companion menu-bar item. See [docs/MACOS_APP.md](docs/MACOS_APP.md) for build and testing notes.

## Development

Run the local checks:

```bash
cargo fmt --check
cargo test
cargo build
python3 -m py_compile scripts/wayland_listener.py
```

Linux builds may need native packages:

```bash
sudo apt update
sudo apt install build-essential pkg-config libdbus-1-dev libxdo-dev libx11-dev libxi-dev libxtst-dev
```

## Documentation

- [RUNNING.md](RUNNING.md): developer/local run instructions.
- [docs/WINDOWS.md](docs/WINDOWS.md): Windows setup, installation, usage, and troubleshooting.
- [docs/RELEASES.md](docs/RELEASES.md): downloadable artifact instructions.
- [docs/MACOS_APP.md](docs/MACOS_APP.md): macOS app wrapper notes.
- [SPEC.md](SPEC.md): product and architecture spec.
- [docs/WAYLAND_POC.md](docs/WAYLAND_POC.md): experimental Linux Wayland listener notes.

## Security Notes

Stringcast handles API keys and text from active fields. Avoid using it in password managers, secure input fields, or sensitive apps. The Linux Wayland Python PoC requires keyboard device access and should be treated as experimental.
