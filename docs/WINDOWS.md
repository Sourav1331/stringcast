# Windows Guide (Stringcast)

This guide explains how to install, configure, run, and use Stringcast on Windows.

Stringcast is currently a Rust CLI/runtime app. It runs in a terminal, listens for text triggers in normal Windows text fields, sends the selected text to the configured AI provider, and replaces the field with the result.

## Current Windows Status

Windows support is implemented for the core runtime path:

- Global input listening through `rdev`
- Foreground app detection through Win32 APIs
- Exclusion checks for apps such as password managers
- Elevated-window blocking when Stringcast is not elevated
- Clipboard-based extraction and replacement
- Static commands such as `?fix`, `?improve`, and `?summarize`
- Dynamic commands such as `?translate:<lang>` and `?ask:<question>`
- API providers: Gemini, OpenAI, Anthropic, and custom OpenAI-compatible APIs
- API key metadata in config and API secrets in Windows Credential Manager through the OS keyring backend

Known remaining Windows hardening items:

- Packaged installer is not available yet.
- Startup-at-login is not wired into the app config yet.
- Focus-change events are not emitted through a Windows foreground-event hook yet.
- Binary signing and SmartScreen hardening are release tasks.

## Requirements

Supported OS:

- Windows 10 64-bit
- Windows 11 64-bit

Required for building from source:

- Rust toolchain through rustup
- Microsoft C++ Build Tools with MSVC
- Windows 10/11 SDK
- Git

If you only use a prebuilt `stringcast.exe`, you do not need Rust or Visual Studio Build Tools.

## Install Development Tools

Install Rust:

```powershell
winget install Rustlang.Rustup
rustup toolchain install stable
rustup default stable
```

Install Visual Studio Build Tools:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools
```

In Visual Studio Installer, select:

- Desktop development with C++
- MSVC v143 or newer
- Windows 10 SDK or Windows 11 SDK

Verify:

```powershell
rustc --version
cargo --version
rustup target list --installed
```

The expected default host target is usually:

```text
x86_64-pc-windows-msvc
```

## Build From Source

From the repository root:

```powershell
cargo build
cargo test
```

For a release binary:

```powershell
cargo build --release
```

The release executable will be:

```text
target\release\stringcast.exe
```

## Optional Portable Install

After building a release binary, copy it to a stable local folder:

```powershell
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Stringcast\bin"
Copy-Item ".\target\release\stringcast.exe" "$env:LOCALAPPDATA\Stringcast\bin\stringcast.exe" -Force
```

Run it directly:

```powershell
& "$env:LOCALAPPDATA\Stringcast\bin\stringcast.exe" status
```

Optional: add it to your user `PATH`:

```powershell
$bin = "$env:LOCALAPPDATA\Stringcast\bin"
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($userPath -split ";") -notcontains $bin) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$bin", "User")
}
```

Open a new terminal after changing `PATH`.

## Initialize Config

Using Cargo:

```powershell
cargo run -- init
cargo run -- show-config
cargo run -- status
```

Using an installed binary:

```powershell
stringcast init
stringcast show-config
stringcast status
```

Default config location:

```text
%APPDATA%\Stringcast\config\config.toml
```

Example:

```text
C:\Users\<you>\AppData\Roaming\Stringcast\config\config.toml
```

## Add an API Key

Set the API key only for the current PowerShell session:

```powershell
$env:STRINGCAST_API_KEY="your-api-key-here"
```

Add key metadata and store the secret in Windows Credential Manager:

```powershell
cargo run -- add-key gemini main "Gemini"
cargo run -- set-provider gemini
cargo run -- api-test
```

Or, if installed:

```powershell
stringcast add-key gemini main "Gemini"
stringcast set-provider gemini
stringcast api-test
```

Supported providers:

```text
gemini
openai
anthropic
custom
```

Provider examples:

```powershell
$env:STRINGCAST_API_KEY="your-gemini-key"
stringcast add-key gemini main "Gemini"
stringcast set-provider gemini
stringcast api-test
```

```powershell
$env:STRINGCAST_API_KEY="your-openai-key"
stringcast add-key openai main "OpenAI"
stringcast set-provider openai
stringcast api-test
```

## Run Stringcast

From source:

```powershell
cargo run -- run
```

Or simply:

```powershell
cargo run
```

Installed binary:

```powershell
stringcast run
```

Leave the terminal open while using Stringcast. Press `Ctrl+C` in that terminal to stop it.

For debugging input and pipeline behavior:

```powershell
$env:STRINGCAST_LOG_EVENTS="1"
cargo run -- run
```

## How To Use It

1. Start Stringcast and leave it running.
2. Open a normal editable text field, such as Notepad, VS Code, a browser textarea, or a chat input.
3. Type text followed by a trigger.
4. Wait for Stringcast to replace the field with the AI output.

Static command examples:

```text
i dont knwo whats happening ?fix
Make this clearer and easier to read ?improve
Turn this into bullet points ?bullets
Summarize this paragraph ?summarize
```

Dynamic command examples:

```text
hello, how are you ?translate:hi
This sounds too direct ?ask:make it sound polite
This is too long ?ask:summarize it in five words
```

Dynamic commands execute when either:

- You stop typing for about 650 ms after a valid dynamic trigger.
- For `?translate:<lang>`, you type a trailing space after the language code.

Examples:

```text
hello world ?translate:es
hello world ?translate:es 
quarterly report ?ask:what are the three key risks
```

## Built-In Commands

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

Language codes use a BCP-47-style subset:

```text
es
hi
ja
fr
de
pt-BR
zh-Hant
```

## Windows Security Behavior

Stringcast uses foreground app detection before executing a command.

Blocked by default:

- Password managers and known sensitive apps from the built-in exclusion list
- Elevated/admin windows when Stringcast itself is not running elevated
- Apps added to your config exclusions

Default Windows exclusions include:

```text
1Password.exe
KeePass.exe
KeePassXC.exe
Bitwarden.exe
LastPass.exe
```

To add your own exclusions, edit:

```text
%APPDATA%\Stringcast\config\config.toml
```

Example:

```toml
[exclusions]
apps = ["MySensitiveApp.exe"]
```

## Startup At Login

Built-in startup-at-login support is not implemented yet. For now, use Task Scheduler if you want Stringcast to start when you sign in.

Suggested Task Scheduler settings:

- Trigger: At log on
- Action: Start a program
- Program: full path to `stringcast.exe`
- Arguments: `run`
- Start in: folder containing `stringcast.exe`

If running from source during development, prefer starting it manually with:

```powershell
cargo run -- run
```

## Troubleshooting

### `Pipeline(Platform(Unavailable))`

This used to indicate missing Windows foreground detection. If it appears again, rebuild the latest code:

```powershell
cargo build
```

Then retry:

```powershell
cargo run -- run
```

### `Pipeline(Extraction(TriggerMissingFromSnapshot))`

This means Stringcast detected the trigger in the keystroke buffer, but the selected/copied text did not contain that trigger. Common causes:

- The target app did not allow normal `Ctrl+A` / `Ctrl+C`.
- The text field changed between trigger detection and extraction.
- Another app or clipboard manager interfered with clipboard contents.

Try Notepad first. If Notepad works but another app fails, that app may be blocking selection or clipboard operations.

### The sentence disappears or the whole field stays selected

This should be prevented by the failed-extraction cleanup path. If it happens:

1. Stop Stringcast with `Ctrl+C`.
2. Rebuild and rerun:

   ```powershell
   cargo build
   cargo run -- run
   ```

3. Retest in Notepad.

### Dynamic commands do not fire

Use one of these patterns:

```text
hello ?translate:hi
hello ?translate:hi 
text ?ask:make it more concise
```

For `?ask:<question>`, stop typing for about 650 ms. For `?translate:<lang>`, either stop typing or type one trailing space after the language code.

### API test fails

Check the active provider and key count:

```powershell
cargo run -- status
```

Re-add the key:

```powershell
$env:STRINGCAST_API_KEY="your-api-key-here"
cargo run -- add-key gemini main "Gemini"
cargo run -- set-provider gemini
cargo run -- api-test
```

### Elevated apps do not work

If the target window is running as Administrator and Stringcast is not, Stringcast blocks the operation. Either:

- Use a non-elevated target app, recommended.
- Run Stringcast elevated only if you understand the security tradeoff.

## Validation Checklist

Use this checklist after building on Windows:

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- check-permissions
cargo run -- api-test
cargo run -- run
```

Manual smoke tests:

- `hello world ?fix` in Notepad
- `hello world ?translate:hi` in Notepad
- `hello world ?ask:make it formal` in Notepad
- Same tests in VS Code
- Same tests in a browser textarea
- Confirm password manager windows are ignored
- Confirm elevated windows are blocked unless Stringcast is also elevated

## Related Files

- `README.md`
- `RUNNING.md`
- `SPEC.md`
- `src/platform/windows.rs`
- `src/input/controller.rs`
- `src/main.rs`
- `src/pipeline.rs`
- `src/runtime.rs`
