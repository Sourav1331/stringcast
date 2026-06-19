use std::env;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use stringcast::api::{
    ApiClient, ApiClientConfig, KeyMaterialStore, KeyPool, ReqwestHttpTransport,
    StaticKeyMaterialStore,
};
use stringcast::input::{InputControllerOutcome, InputEvent, InputHook, RdevInputHook};
use stringcast::platform::{PermissionChecker, SystemPermissionChecker};
use stringcast::runtime::StringcastRuntime;
use stringcast::storage::{config_file_path, ApiKeyConfig, AppConfig, KeyringKeyMaterialStore};

const INPUT_EVENT_QUEUE_CAPACITY: usize = 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("Stringcast failed to start: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("init") => return init_config(),
        Some("status") => return status(),
        Some("enable") => return set_enabled(true),
        Some("disable") => return set_enabled(false),
        Some("set-provider") => return set_provider(&args[1..]),
        Some("set-model") => return set_model(&args[1..]),
        Some("show-config") => return show_config_path(),
        Some("check-permissions") => return check_permissions(),
        Some("request-permissions") => return request_permissions(),
        Some("add-key") => return add_key(&args[1..]),
        Some("api-test") => return api_test(),
        Some("run") | None => {}
        Some(command) => return Err(format!("unknown command: {command}\n{}", usage())),
    }

    let config_path =
        config_file_path().map_err(|error| format!("config path error: {error:?}"))?;
    ensure_config_exists(&config_path)?;
    let config = AppConfig::load(&config_path).map_err(|error| {
        format!(
            "could not load config from {}: {error:?}",
            config_path.display()
        )
    })?;

    launch_app_menu_if_needed();
    request_app_permissions_if_needed();

    if !skip_permission_preflight() {
        preflight_permissions()?;
    }

    let key_material = load_key_material(&config)?;
    let mut runtime = StringcastRuntime::from_config(&config, key_material)
        .map_err(|error| format!("runtime build error: {error:?}"))?;

    println!("Stringcast running. Config: {}", config_path.display());

    let mut hook = input_hook();
    let log_events = log_events();
    let (event_sender, event_receiver) = mpsc::sync_channel(INPUT_EVENT_QUEUE_CAPACITY);
    let worker_log_events = log_events;
    let _input_worker = thread::spawn(move || loop {
        let received = match runtime.pending_dynamic_deadline() {
            Some(deadline) => {
                let timeout = deadline.saturating_duration_since(Instant::now());
                match event_receiver.recv_timeout(timeout) {
                    Ok(received) => received,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        log_runtime_outcome(
                            runtime.handle_pending_timeout(Instant::now()),
                            worker_log_events,
                        );
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match event_receiver.recv() {
                Ok(received) => received,
                Err(_) => break,
            },
        };

        let (event, received_at) = received;
        log_runtime_outcome(runtime.handle_event(event, received_at), worker_log_events);
    });

    hook.run(move |event| {
        let received_at = Instant::now();
        if log_events {
            eprintln!("Stringcast input event: {}", describe_input_event(&event));
        }

        match event_sender.try_send((event, received_at)) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                if log_events {
                    eprintln!("Stringcast event dropped: input worker queue full");
                }
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                eprintln!("Stringcast event error: input worker stopped");
            }
        }
    })
    .map_err(|error| format!("input hook error: {error:?}"))
}

fn log_runtime_outcome(
    outcome: Result<InputControllerOutcome, stringcast::input::InputControllerError>,
    log_events: bool,
) {
    match outcome {
        Ok(outcome) if log_events => {
            eprintln!(
                "Stringcast input outcome: {}",
                describe_input_outcome(&outcome)
            );
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!("Stringcast event error: {error:?}");
        }
    }
}

fn input_hook() -> RdevInputHook {
    RdevInputHook::new()
}

fn ensure_config_exists(config_path: &Path) -> Result<(), String> {
    if config_path.exists() {
        return Ok(());
    }

    let config = AppConfig::default();
    config
        .save_atomic(config_path)
        .map_err(|error| format!("could not create default config: {error:?}"))?;
    println!("Created default config: {}", config_path.display());
    Ok(())
}

fn preflight_permissions() -> Result<(), String> {
    let report = SystemPermissionChecker::default().permission_report();
    report.startup_error_message().map_or(Ok(()), Err)
}

fn skip_permission_preflight() -> bool {
    env::var("STRINGCAST_SKIP_PERMISSION_PREFLIGHT").is_ok_and(|value| value == "1")
        || running_from_app_bundle()
}

fn log_events() -> bool {
    env::var("STRINGCAST_LOG_EVENTS").is_ok_and(|value| value == "1")
}

fn describe_input_event(event: &InputEvent) -> String {
    match event {
        InputEvent::Text(text) => format!("Text(len={})", text.chars().count()),
        InputEvent::Backspace => "Backspace".to_string(),
        InputEvent::Delete => "Delete".to_string(),
        InputEvent::Enter => "Enter".to_string(),
        InputEvent::Escape => "Escape".to_string(),
        InputEvent::Tab => "Tab".to_string(),
        InputEvent::Navigation(key) => format!("Navigation({key:?})"),
        InputEvent::MouseButton => "MouseButton".to_string(),
        InputEvent::Shortcut(shortcut) => format!("Shortcut({shortcut:?})"),
        InputEvent::FocusChanged => "FocusChanged".to_string(),
        InputEvent::SleepOrLock => "SleepOrLock".to_string(),
    }
}

fn describe_input_outcome(outcome: &InputControllerOutcome) -> String {
    match outcome {
        InputControllerOutcome::IgnoredSynthetic => "IgnoredSynthetic".to_string(),
        InputControllerOutcome::BufferUpdated(buffer) => {
            format!("BufferUpdated(len={})", buffer.chars().count())
        }
        InputControllerOutcome::BufferCleared => "BufferCleared".to_string(),
        InputControllerOutcome::Pipeline(outcome) => format!("Pipeline({outcome:?})"),
    }
}

#[cfg(target_os = "macos")]
fn running_from_app_bundle() -> bool {
    env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .is_some_and(|path| path.contains(".app/Contents/MacOS/"))
}

#[cfg(not(target_os = "macos"))]
fn running_from_app_bundle() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn launch_app_menu_if_needed() {
    if !running_from_app_bundle() || env::var("STRINGCAST_APP_MENU_DISABLED").is_ok() {
        return;
    }

    let Ok(exe) = env::current_exe() else {
        return;
    };
    let Some(macos_dir) = exe.parent() else {
        return;
    };

    let menu_exe = macos_dir.join("StringcastMenu");
    if !menu_exe.exists() {
        return;
    }

    let _ = std::process::Command::new(menu_exe)
        .env("STRINGCAST_APP_RUNTIME_PID", std::process::id().to_string())
        .spawn();
}

#[cfg(not(target_os = "macos"))]
fn launch_app_menu_if_needed() {}

#[cfg(target_os = "macos")]
fn request_app_permissions_if_needed() {
    if running_from_app_bundle() {
        let _ = stringcast::platform::request_accessibility_permission();
    }
}

#[cfg(not(target_os = "macos"))]
fn request_app_permissions_if_needed() {}

fn check_permissions() -> Result<(), String> {
    let report = SystemPermissionChecker::default().permission_report();
    println!("Accessibility: {:?}", report.accessibility);
    println!("Input Monitoring: {:?}", report.input_monitoring);
    if let Some(message) = report.startup_error_message() {
        println!("{message}");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn request_permissions() -> Result<(), String> {
    let trusted = stringcast::platform::request_accessibility_permission();
    let report = SystemPermissionChecker::default().permission_report();
    println!("Accessibility prompt requested: {trusted}");
    println!("Accessibility: {:?}", report.accessibility);
    println!("Input Monitoring: {:?}", report.input_monitoring);
    println!(
        "If macOS opens Privacy & Security, enable Stringcast or stringcast, then restart Stringcast."
    );
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn request_permissions() -> Result<(), String> {
    check_permissions()
}

fn init_config() -> Result<(), String> {
    let config_path =
        config_file_path().map_err(|error| format!("config path error: {error:?}"))?;
    let config = AppConfig::load(&config_path).map_err(|error| {
        format!(
            "could not load config from {}: {error:?}",
            config_path.display()
        )
    })?;
    config
        .save_atomic(&config_path)
        .map_err(|error| format!("could not save config: {error:?}"))?;
    println!("Initialized config: {}", config_path.display());
    Ok(())
}

fn show_config_path() -> Result<(), String> {
    let config_path =
        config_file_path().map_err(|error| format!("config path error: {error:?}"))?;
    println!("{}", config_path.display());
    Ok(())
}

fn status() -> Result<(), String> {
    let config_path =
        config_file_path().map_err(|error| format!("config path error: {error:?}"))?;
    let config = AppConfig::load(&config_path).map_err(|error| {
        format!(
            "could not load config from {}: {error:?}",
            config_path.display()
        )
    })?;

    println!("Config: {}", config_path.display());
    println!("Enabled: {}", config.general.enabled);
    println!("Active provider: {}", config.provider.active);
    println!(
        "Active provider keys: {}",
        config.active_provider_key_count()
    );
    println!("Custom commands: {}", config.commands.custom.len());
    println!(
        "Disabled built-ins: {}",
        config.commands.disabled_builtins.len()
    );
    Ok(())
}

fn set_enabled(enabled: bool) -> Result<(), String> {
    let config_path =
        config_file_path().map_err(|error| format!("config path error: {error:?}"))?;
    let mut config = AppConfig::load(&config_path).map_err(|error| {
        format!(
            "could not load config from {}: {error:?}",
            config_path.display()
        )
    })?;

    config.set_enabled(enabled);
    config
        .save_atomic(&config_path)
        .map_err(|error| format!("could not save config: {error:?}"))?;
    println!("Stringcast enabled: {enabled}");
    Ok(())
}

fn set_provider(args: &[String]) -> Result<(), String> {
    let Some(provider) = args.first() else {
        return Err(format!("set-provider requires <provider>\n{}", usage()));
    };

    let config_path =
        config_file_path().map_err(|error| format!("config path error: {error:?}"))?;
    let mut config = AppConfig::load(&config_path).map_err(|error| {
        format!(
            "could not load config from {}: {error:?}",
            config_path.display()
        )
    })?;

    config
        .set_provider(provider)
        .map_err(|error| format!("invalid provider: {error:?}"))?;
    config
        .save_atomic(&config_path)
        .map_err(|error| format!("could not save config: {error:?}"))?;
    println!("Active provider: {provider}");
    Ok(())
}

fn set_model(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err(format!(
            "set-model requires <provider> <model>\n{}",
            usage()
        ));
    }

    let provider = &args[0];
    let model = &args[1];
    let config_path =
        config_file_path().map_err(|error| format!("config path error: {error:?}"))?;
    let mut config = AppConfig::load(&config_path).map_err(|error| {
        format!(
            "could not load config from {}: {error:?}",
            config_path.display()
        )
    })?;

    config
        .set_provider_model(provider, model)
        .map_err(|error| format!("invalid model setting: {error:?}"))?;
    config.provider.active = provider.to_string();
    config
        .save_atomic(&config_path)
        .map_err(|error| format!("could not save config: {error:?}"))?;
    println!("Active provider: {provider}");
    println!("{provider} model: {model}");
    Ok(())
}

fn add_key(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err(format!(
            "add-key requires <provider> <key-id> [alias]\n{}",
            usage()
        ));
    }

    let provider = args[0].as_str();
    if !matches!(provider, "gemini" | "openai" | "anthropic" | "custom") {
        return Err("provider must be one of: gemini, openai, anthropic, custom".to_string());
    }

    let key_id = args[1].clone();
    let alias = args.get(2).cloned();
    let secret = env::var("STRINGCAST_API_KEY")
        .map_err(|_| "set STRINGCAST_API_KEY before running add-key".to_string())?;

    let config_path =
        config_file_path().map_err(|error| format!("config path error: {error:?}"))?;
    let mut config = AppConfig::load(&config_path).map_err(|error| {
        format!(
            "could not load config from {}: {error:?}",
            config_path.display()
        )
    })?;

    KeyringKeyMaterialStore
        .set_key(provider, &key_id, &secret)
        .map_err(|error| format!("could not store key in OS keychain: {error:?}"))?;

    config.upsert_api_key(ApiKeyConfig {
        id: key_id.clone(),
        provider: provider.to_string(),
        alias,
        status: "Active".to_string(),
        created_at: unix_timestamp_string(),
    });
    config.provider.active = provider.to_string();
    config
        .save_atomic(&config_path)
        .map_err(|error| format!("could not save config: {error:?}"))?;

    println!(
        "Stored key metadata for {provider}/{key_id} in {}",
        config_path.display()
    );
    Ok(())
}

fn api_test() -> Result<(), String> {
    let config_path =
        config_file_path().map_err(|error| format!("config path error: {error:?}"))?;
    let config = AppConfig::load(&config_path).map_err(|error| {
        format!(
            "could not load config from {}: {error:?}",
            config_path.display()
        )
    })?;
    let api_config = ApiClientConfig::try_from(&config)
        .map_err(|error| format!("api config error: {error:?}"))?;
    let key_pool = KeyPool::from_config(&config.api_keys)
        .map_err(|error| format!("key pool error: {error}"))?;
    let transport = ReqwestHttpTransport::new(std::time::Duration::from_millis(
        config.api.response_timeout_ms,
    ))
    .map_err(|error| format!("transport build error: {error}"))?;

    println!(
        "Testing provider={} model={} keys={}",
        config.provider.active,
        api_config.model,
        key_pool.keys().len()
    );

    let key_material = load_key_material(&config)?;
    let mut client = ApiClient::new(api_config, key_pool, transport, key_material);
    match client.transform("Reply with OK only.", "health check", false, Instant::now()) {
        Ok(output) => {
            println!("API test succeeded: {output}");
            Ok(())
        }
        Err(error) => Err(format!("API test failed: {error}")),
    }
}

fn load_key_material(config: &AppConfig) -> Result<StaticKeyMaterialStore, String> {
    let keyring = KeyringKeyMaterialStore;
    let mut keys = Vec::new();

    for key in config.api_keys.iter().filter(|key| {
        key.provider == config.provider.active && key.status.eq_ignore_ascii_case("active")
    }) {
        let Some(secret) = keyring.key_material(&key.id) else {
            return Err(format!(
                "missing keychain secret for {}/{}; re-add it with STRINGCAST_API_KEY=<secret> cargo run -- add-key {} {}",
                key.provider, key.id, key.provider, key.id
            ));
        };
        keys.push((key.id.clone(), secret));
    }

    if keys.is_empty() {
        return Err(format!(
            "no active API key configured for provider '{}'",
            config.provider.active
        ));
    }

    Ok(StaticKeyMaterialStore::new(keys))
}

fn unix_timestamp_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn usage() -> &'static str {
    "usage:
  stringcast run
  stringcast init
  stringcast status
  stringcast enable
  stringcast disable
  stringcast set-provider <gemini|openai|anthropic|custom>
  stringcast set-model <gemini|openai|anthropic|custom> <model>
  stringcast show-config
  stringcast check-permissions
  stringcast request-permissions
  stringcast api-test
  STRINGCAST_API_KEY=<secret> stringcast add-key <gemini|openai|anthropic|custom> <key-id> [alias]"
}
