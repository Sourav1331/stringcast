use crate::api::{
    ApiClient, ApiClientConfig, ApiTextTransformer, KeyMaterialStore, KeyPool,
    ReqwestHttpTransport, ReqwestTransportBuildError,
};
use crate::clipboard::{ArboardClipboard, ClipboardError};
use crate::commands::{CommandRegistry, CustomCommandError};
use crate::extraction::ClipboardTextExtractor;
use crate::input::{
    EnigoInputSimulator, GuardedInputSimulator, InputController, InputControllerError, InputEvent,
    InputSimulationError, SyntheticInputGuard,
};
use crate::pipeline::TransformationPipeline;
use crate::platform::{ExclusionMatcher, OperationGate};
use crate::replacement::ClipboardTextReplacer;
use crate::storage::AppConfig;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use crate::platform::MacOsForegroundAppProvider as SystemForegroundAppProvider;

#[cfg(target_os = "windows")]
use crate::platform::WindowsForegroundAppProvider as SystemForegroundAppProvider;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use crate::platform::{ForegroundApp, StaticForegroundAppProvider as SystemForegroundAppProvider};

// Use Enigo-based simulator as the default system input simulator across platforms.
type SystemInputSimulator = GuardedInputSimulator<EnigoInputSimulator>;
type SystemExtractor = ClipboardTextExtractor<ArboardClipboard, SystemInputSimulator>;
type SystemReplacer = ClipboardTextReplacer<ArboardClipboard, SystemInputSimulator>;
type SystemTransport = ReqwestHttpTransport;
type SystemTransformer<K> = ApiTextTransformer<SystemTransport, K>;
type SystemController<K> = InputController<
    SystemExtractor,
    SystemTransformer<K>,
    SystemReplacer,
    SystemForegroundAppProvider,
>;

#[derive(Debug, Clone)]
pub struct RuntimePlan {
    pub registry: CommandRegistry,
    pub key_pool: KeyPool,
    pub api_config: ApiClientConfig,
    pub gate: OperationGate,
    pub response_timeout: Duration,
    pub show_spinner: bool,
    pub select_all_wait: Duration,
    pub clipboard_read_wait: Duration,
    pub clipboard_restore_wait: Duration,
}

#[derive(Debug)]
pub enum RuntimeBuildError {
    Config(String),
    Commands(CustomCommandError),
    ApiConfig(crate::api::ProviderError),
    KeyPool(String),
    Transport(ReqwestTransportBuildError),
    Clipboard(ClipboardError),
    Input(InputSimulationError),
}

pub struct StringcastRuntime<K> {
    controller: SystemController<K>,
}

impl RuntimePlan {
    pub fn from_config(config: &AppConfig) -> Result<Self, RuntimeBuildError> {
        config
            .validate()
            .map_err(|error| RuntimeBuildError::Config(format!("{error:?}")))?;

        Ok(Self {
            registry: CommandRegistry::from_config(&config.commands)
                .map_err(RuntimeBuildError::Commands)?,
            key_pool: KeyPool::from_config(&config.api_keys).map_err(RuntimeBuildError::KeyPool)?,
            api_config: ApiClientConfig::try_from(config).map_err(RuntimeBuildError::ApiConfig)?,
            gate: OperationGate::new(
                config.general.enabled,
                ExclusionMatcher::from_config(&config.exclusions),
            ),
            response_timeout: Duration::from_millis(config.api.response_timeout_ms),
            show_spinner: config.general.show_spinner,
            select_all_wait: Duration::from_millis(config.extraction.select_all_wait_ms),
            clipboard_read_wait: Duration::from_millis(config.extraction.clipboard_read_wait_ms),
            clipboard_restore_wait: Duration::from_millis(
                config.extraction.clipboard_restore_wait_ms,
            ),
        })
    }
}

impl<K> StringcastRuntime<K>
where
    K: KeyMaterialStore,
{
    pub fn from_config(config: &AppConfig, key_material: K) -> Result<Self, RuntimeBuildError> {
        Self::from_plan(RuntimePlan::from_config(config)?, key_material)
    }

    pub fn from_plan(plan: RuntimePlan, key_material: K) -> Result<Self, RuntimeBuildError> {
        let guard = SyntheticInputGuard::default();
        let transport = ReqwestHttpTransport::new(plan.response_timeout)
            .map_err(RuntimeBuildError::Transport)?;
        let api_client = ApiClient::new(plan.api_config, plan.key_pool, transport, key_material);
        let transformer = ApiTextTransformer::new(api_client);

        let extractor = ClipboardTextExtractor::with_delays(
            ArboardClipboard::new().map_err(RuntimeBuildError::Clipboard)?,
            GuardedInputSimulator::new(
                EnigoInputSimulator::new().map_err(RuntimeBuildError::Input)?,
                guard.clone(),
            ),
            plan.select_all_wait,
            plan.clipboard_read_wait,
        );
        let replacer = ClipboardTextReplacer::with_delays(
            ArboardClipboard::new().map_err(RuntimeBuildError::Clipboard)?,
            GuardedInputSimulator::new(
                EnigoInputSimulator::new().map_err(RuntimeBuildError::Input)?,
                guard.clone(),
            ),
            plan.select_all_wait,
            plan.clipboard_restore_wait,
        );
        let pipeline = if plan.show_spinner {
            TransformationPipeline::with_progress_marker(
                plan.registry,
                extractor,
                transformer,
                replacer,
                "[...]",
            )
        } else {
            TransformationPipeline::new(plan.registry, extractor, transformer, replacer)
        };

        Ok(Self {
            controller: InputController::new(
                pipeline,
                system_foreground_provider(),
                plan.gate,
                guard,
            ),
        })
    }

    pub fn handle_event(
        &mut self,
        event: InputEvent,
        now: Instant,
    ) -> Result<crate::input::InputControllerOutcome, InputControllerError> {
        self.controller.handle_event(event, now)
    }

    pub fn handle_pending_timeout(
        &mut self,
        now: Instant,
    ) -> Result<crate::input::InputControllerOutcome, InputControllerError> {
        self.controller.handle_pending_timeout(now)
    }

    pub fn pending_dynamic_deadline(&self) -> Option<Instant> {
        self.controller.pending_dynamic_deadline()
    }

    pub fn buffer(&self) -> &str {
        self.controller.buffer()
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn system_foreground_provider() -> SystemForegroundAppProvider {
    SystemForegroundAppProvider::new()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_foreground_provider() -> SystemForegroundAppProvider {
    SystemForegroundAppProvider::new(ForegroundApp {
        app_id: "unknown".to_string(),
        window_id: None,
        display_name: None,
        secure_input: false,
        elevated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::ApiKeyConfig;

    #[test]
    fn runtime_plan_builds_from_valid_config() {
        let mut config = AppConfig::default();
        config.provider.active = "openai".to_string();
        config.api_keys.push(ApiKeyConfig {
            id: "key-1".to_string(),
            provider: "openai".to_string(),
            alias: Some("Main".to_string()),
            status: "Active".to_string(),
            created_at: "2026-05-31T00:00:00Z".to_string(),
        });

        let plan = RuntimePlan::from_config(&config).unwrap();

        assert_eq!(plan.api_config.model, "gpt-4o-mini");
        assert_eq!(plan.key_pool.keys().len(), 1);
        assert_eq!(plan.response_timeout, Duration::from_millis(30_000));
        assert!(plan.show_spinner);
        assert_eq!(plan.select_all_wait, Duration::from_millis(100));
        assert_eq!(plan.clipboard_read_wait, Duration::from_millis(300));
        assert_eq!(plan.clipboard_restore_wait, Duration::from_millis(150));
    }

    #[test]
    fn runtime_plan_rejects_invalid_custom_command() {
        let mut config = AppConfig::default();
        config.commands.custom.push(crate::storage::CommandConfig {
            trigger: "?ask:bad".to_string(),
            name: "Bad".to_string(),
            prompt: "Bad: {text}".to_string(),
            enabled: true,
            case_sensitive: true,
            raw_output: false,
            created_at: "2026-05-31T00:00:00Z".to_string(),
        });

        let result = RuntimePlan::from_config(&config);

        assert!(matches!(result, Err(RuntimeBuildError::Commands(_))));
    }
}
