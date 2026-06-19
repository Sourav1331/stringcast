use crate::commands::{CommandDefinition, CommandRegistry};
use crate::detection::{detect_trigger, finalize_pending_dynamic, DetectionDecision, TriggerMatch};
use crate::extraction::{ExtractionContext, ExtractionError, TextExtractor};
use crate::orchestrator::{OperationOrchestrator, OrchestratorError};
use crate::platform::{
    ForegroundAppProvider, OperationGate, OperationGateDecision, PlatformContextError,
};
use crate::replacement::{ReplacementError, TextReplacer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformError {
    ApiUnavailable,
    EmptyOutput,
    ProviderRejected,
}

pub trait TextTransformer {
    fn transform(
        &mut self,
        command: &CommandDefinition,
        input: &str,
    ) -> Result<String, TransformError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    Orchestrator(OrchestratorError),
    Extraction(ExtractionError),
    Transform(TransformError),
    Replacement(ReplacementError),
    Platform(PlatformContextError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineOutcome {
    NoMatch,
    PendingDynamic,
    Blocked(OperationGateDecision),
    Replaced {
        trigger_text: String,
        replacement_text: String,
    },
}

impl From<OrchestratorError> for PipelineError {
    fn from(error: OrchestratorError) -> Self {
        Self::Orchestrator(error)
    }
}

impl From<ExtractionError> for PipelineError {
    fn from(error: ExtractionError) -> Self {
        Self::Extraction(error)
    }
}

impl From<TransformError> for PipelineError {
    fn from(error: TransformError) -> Self {
        Self::Transform(error)
    }
}

impl From<ReplacementError> for PipelineError {
    fn from(error: ReplacementError) -> Self {
        Self::Replacement(error)
    }
}

impl From<PlatformContextError> for PipelineError {
    fn from(error: PlatformContextError) -> Self {
        Self::Platform(error)
    }
}

pub struct TransformationPipeline<E, T, R> {
    registry: CommandRegistry,
    orchestrator: OperationOrchestrator,
    extractor: E,
    transformer: T,
    replacer: R,
    progress_marker: Option<String>,
}

impl<E, T, R> TransformationPipeline<E, T, R>
where
    E: TextExtractor,
    T: TextTransformer,
    R: TextReplacer,
{
    pub fn new(registry: CommandRegistry, extractor: E, transformer: T, replacer: R) -> Self {
        Self {
            registry,
            orchestrator: OperationOrchestrator::new(),
            extractor,
            transformer,
            replacer,
            progress_marker: None,
        }
    }

    pub fn with_progress_marker(
        registry: CommandRegistry,
        extractor: E,
        transformer: T,
        replacer: R,
        progress_marker: impl Into<String>,
    ) -> Self {
        Self {
            registry,
            orchestrator: OperationOrchestrator::new(),
            extractor,
            transformer,
            replacer,
            progress_marker: Some(progress_marker.into()),
        }
    }

    pub fn process_buffer(
        &mut self,
        buffer: &str,
        app_id: impl Into<String>,
        window_id: Option<String>,
    ) -> Result<PipelineOutcome, PipelineError> {
        match detect_trigger(buffer, &self.registry) {
            DetectionDecision::NoMatch => Ok(PipelineOutcome::NoMatch),
            DetectionDecision::PendingDynamic(_) => Ok(PipelineOutcome::PendingDynamic),
            DetectionDecision::Matched(trigger_match) => {
                self.execute_match(trigger_match, app_id.into(), window_id)
            }
        }
    }

    pub fn finalize_pending_buffer(
        &mut self,
        buffer: &str,
        app_id: impl Into<String>,
        window_id: Option<String>,
    ) -> Result<PipelineOutcome, PipelineError> {
        match finalize_pending_dynamic(buffer) {
            DetectionDecision::NoMatch => Ok(PipelineOutcome::NoMatch),
            DetectionDecision::PendingDynamic(_) => Ok(PipelineOutcome::PendingDynamic),
            DetectionDecision::Matched(trigger_match) => {
                self.execute_match(trigger_match, app_id.into(), window_id)
            }
        }
    }

    pub fn process_foreground_buffer<P>(
        &mut self,
        buffer: &str,
        provider: &mut P,
        gate: &OperationGate,
    ) -> Result<PipelineOutcome, PipelineError>
    where
        P: ForegroundAppProvider,
    {
        let app = provider.foreground_app()?;
        let decision = gate.evaluate(&app);
        if decision != OperationGateDecision::Allow {
            return Ok(PipelineOutcome::Blocked(decision));
        }

        self.process_buffer(buffer, app.app_id, app.window_id)
    }

    pub fn finalize_pending_foreground_buffer<P>(
        &mut self,
        buffer: &str,
        provider: &mut P,
        gate: &OperationGate,
    ) -> Result<PipelineOutcome, PipelineError>
    where
        P: ForegroundAppProvider,
    {
        let app = provider.foreground_app()?;
        let decision = gate.evaluate(&app);
        if decision != OperationGateDecision::Allow {
            return Ok(PipelineOutcome::Blocked(decision));
        }

        self.finalize_pending_buffer(buffer, app.app_id, app.window_id)
    }

    pub fn into_parts(self) -> (OperationOrchestrator, E, T, R) {
        (
            self.orchestrator,
            self.extractor,
            self.transformer,
            self.replacer,
        )
    }

    fn execute_match(
        &mut self,
        trigger_match: TriggerMatch,
        app_id: String,
        window_id: Option<String>,
    ) -> Result<PipelineOutcome, PipelineError> {
        let operation_id = self.orchestrator.begin_static_extraction()?;
        let snapshot = match self.extractor.extract(ExtractionContext {
            operation_id,
            app_id,
            window_id,
            trigger_match: trigger_match.clone(),
        }) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.orchestrator.fail();
                return Err(PipelineError::Extraction(error));
            }
        };
        if let Err(error) = self.orchestrator.complete_extraction(snapshot.clone()) {
            self.orchestrator.fail();
            return Err(PipelineError::Orchestrator(error));
        }

        let progress_text = self.progress_text(&snapshot);
        let current_text = progress_text
            .as_deref()
            .unwrap_or(&snapshot.replacement_target_text);
        if let Some(progress_text) = progress_text.as_ref() {
            if let Err(error) = self.replacer.replace_current(
                &snapshot,
                &snapshot.replacement_target_text,
                progress_text,
            ) {
                self.orchestrator.fail();
                return Err(PipelineError::Replacement(error));
            }
        }

        let replacement_text = match self
            .transformer
            .transform(&trigger_match.command, &snapshot.transform_input)
        {
            Ok(replacement_text) => replacement_text,
            Err(error) => {
                self.restore_after_progress(&snapshot, current_text);
                self.orchestrator.fail();
                return Err(PipelineError::Transform(error));
            }
        };
        if replacement_text.trim().is_empty() {
            self.restore_after_progress(&snapshot, current_text);
            self.orchestrator.fail();
            return Err(PipelineError::Transform(TransformError::EmptyOutput));
        }

        if let Err(error) = self.orchestrator.begin_replacement() {
            self.orchestrator.fail();
            return Err(PipelineError::Orchestrator(error));
        }
        if let Err(error) =
            self.replacer
                .replace_current(&snapshot, current_text, &replacement_text)
        {
            self.orchestrator.fail();
            return Err(PipelineError::Replacement(error));
        }
        if let Err(error) = self.orchestrator.begin_verification() {
            self.orchestrator.fail();
            return Err(PipelineError::Orchestrator(error));
        }
        if let Err(error) = self.orchestrator.complete() {
            self.orchestrator.fail();
            return Err(PipelineError::Orchestrator(error));
        }

        Ok(PipelineOutcome::Replaced {
            trigger_text: snapshot.trigger_text,
            replacement_text,
        })
    }

    fn progress_text(&self, snapshot: &crate::orchestrator::OperationSnapshot) -> Option<String> {
        self.progress_marker
            .as_ref()
            .map(|marker| format!("{} {marker}", snapshot.replacement_target_text))
    }

    fn restore_after_progress(
        &mut self,
        snapshot: &crate::orchestrator::OperationSnapshot,
        current_text: &str,
    ) {
        if self.progress_marker.is_some() {
            let _ = self.replacer.replace_current(
                snapshot,
                current_text,
                &snapshot.replacement_target_text,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandKind;
    use crate::extraction::{BufferTextExtractor, ExtractionContext};
    use crate::platform::{
        ExclusionMatcher, ForegroundApp, OperationGate, OperationGateDecision,
        StaticForegroundAppProvider,
    };
    use crate::replacement::NoopTextReplacer;

    #[derive(Debug, Clone)]
    struct FakeTransformer {
        output: Result<String, TransformError>,
        calls: Vec<(CommandKind, String)>,
    }

    impl TextTransformer for FakeTransformer {
        fn transform(
            &mut self,
            command: &CommandDefinition,
            input: &str,
        ) -> Result<String, TransformError> {
            self.calls.push((command.kind.clone(), input.to_string()));
            self.output.clone()
        }
    }

    #[derive(Debug, Clone)]
    struct FailingExtractor {
        error: ExtractionError,
    }

    impl TextExtractor for FailingExtractor {
        fn extract(
            &mut self,
            _context: ExtractionContext,
        ) -> Result<crate::orchestrator::OperationSnapshot, ExtractionError> {
            Err(self.error.clone())
        }
    }

    #[test]
    fn full_pipeline_replaces_static_trigger() {
        let mut pipeline = TransformationPipeline::new(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer {
                output: Ok("I do not know.".to_string()),
                calls: vec![],
            },
            NoopTextReplacer::default(),
        );

        let outcome = pipeline
            .process_buffer(
                "i dont no ?fix",
                "com.example.App",
                Some("window-1".to_string()),
            )
            .unwrap();
        let (_, _, transformer, replacer) = pipeline.into_parts();

        assert_eq!(
            outcome,
            PipelineOutcome::Replaced {
                trigger_text: "?fix".to_string(),
                replacement_text: "I do not know.".to_string()
            }
        );
        assert_eq!(transformer.calls[0].1, "i dont no");
        assert_eq!(replacer.replacements[0].1, "I do not know.");
    }

    #[test]
    fn pipeline_reports_pending_dynamic_without_side_effects() {
        let mut pipeline = TransformationPipeline::new(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer {
                output: Ok("Hola".to_string()),
                calls: vec![],
            },
            NoopTextReplacer::default(),
        );

        let outcome = pipeline
            .process_buffer("hello ?translate:es", "com.example.App", None)
            .unwrap();
        let (_, _, transformer, replacer) = pipeline.into_parts();

        assert_eq!(outcome, PipelineOutcome::PendingDynamic);
        assert!(transformer.calls.is_empty());
        assert!(replacer.replacements.is_empty());
    }

    #[test]
    fn pipeline_finalizes_pending_dynamic_trigger() {
        let mut pipeline = TransformationPipeline::new(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer {
                output: Ok("Hola".to_string()),
                calls: vec![],
            },
            NoopTextReplacer::default(),
        );

        let outcome = pipeline
            .finalize_pending_buffer("hello ?translate:es", "com.example.App", None)
            .unwrap();
        let (_, _, transformer, replacer) = pipeline.into_parts();

        assert_eq!(
            outcome,
            PipelineOutcome::Replaced {
                trigger_text: "?translate:es".to_string(),
                replacement_text: "Hola".to_string()
            }
        );
        assert_eq!(transformer.calls.len(), 1);
        assert_eq!(replacer.replacements[0].1, "Hola");
    }

    #[test]
    fn transform_failure_does_not_replace() {
        let mut pipeline = TransformationPipeline::new(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer {
                output: Err(TransformError::ProviderRejected),
                calls: vec![],
            },
            NoopTextReplacer::default(),
        );

        let result = pipeline.process_buffer("hello ?fix", "com.example.App", None);
        let (_, _, _, replacer) = pipeline.into_parts();

        assert_eq!(
            result,
            Err(PipelineError::Transform(TransformError::ProviderRejected))
        );
        assert!(replacer.replacements.is_empty());
    }

    #[test]
    fn progress_marker_is_replaced_by_final_output() {
        let mut pipeline = TransformationPipeline::with_progress_marker(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer {
                output: Ok("I do not know.".to_string()),
                calls: vec![],
            },
            NoopTextReplacer::default(),
            "[Stringcast working...]",
        );

        let outcome = pipeline
            .process_buffer("i dont no ?fix", "com.example.App", None)
            .unwrap();
        let (_, _, _, replacer) = pipeline.into_parts();

        assert_eq!(
            outcome,
            PipelineOutcome::Replaced {
                trigger_text: "?fix".to_string(),
                replacement_text: "I do not know.".to_string()
            }
        );
        assert_eq!(replacer.replacements.len(), 2);
        assert_eq!(
            replacer.replacements[0].1,
            "i dont no ?fix [Stringcast working...]"
        );
        assert_eq!(replacer.replacements[1].1, "I do not know.");
    }

    #[test]
    fn progress_marker_is_restored_on_transform_failure() {
        let mut pipeline = TransformationPipeline::with_progress_marker(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer {
                output: Err(TransformError::ProviderRejected),
                calls: vec![],
            },
            NoopTextReplacer::default(),
            "[Stringcast working...]",
        );

        let result = pipeline.process_buffer("hello ?fix", "com.example.App", None);
        let (_, _, _, replacer) = pipeline.into_parts();

        assert_eq!(
            result,
            Err(PipelineError::Transform(TransformError::ProviderRejected))
        );
        assert_eq!(replacer.replacements.len(), 2);
        assert_eq!(
            replacer.replacements[0].1,
            "hello ?fix [Stringcast working...]"
        );
        assert_eq!(replacer.replacements[1].1, "hello ?fix");
    }

    #[test]
    fn extraction_failure_resets_active_operation() {
        let mut pipeline = TransformationPipeline::new(
            CommandRegistry::new(),
            FailingExtractor {
                error: ExtractionError::TriggerMissingFromSnapshot,
            },
            FakeTransformer {
                output: Ok("hello".to_string()),
                calls: vec![],
            },
            NoopTextReplacer::default(),
        );

        let first = pipeline.process_buffer("hello ?fix", "com.example.App", None);
        let second = pipeline.process_buffer("hello ?fix", "com.example.App", None);

        assert_eq!(
            first,
            Err(PipelineError::Extraction(
                ExtractionError::TriggerMissingFromSnapshot
            ))
        );
        assert_eq!(
            second,
            Err(PipelineError::Extraction(
                ExtractionError::TriggerMissingFromSnapshot
            ))
        );
    }

    #[test]
    fn foreground_pipeline_blocks_excluded_apps_before_transforming() {
        let mut pipeline = TransformationPipeline::new(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer {
                output: Ok("hello".to_string()),
                calls: vec![],
            },
            NoopTextReplacer::default(),
        );
        let mut provider = StaticForegroundAppProvider::new(ForegroundApp {
            app_id: "1Password.exe".to_string(),
            window_id: None,
            display_name: None,
            secure_input: false,
            elevated: false,
        });
        let gate = OperationGate::new(true, ExclusionMatcher::from_config(&Default::default()));

        let outcome = pipeline
            .process_foreground_buffer("secret ?fix", &mut provider, &gate)
            .unwrap();
        let (_, _, transformer, replacer) = pipeline.into_parts();

        assert_eq!(
            outcome,
            PipelineOutcome::Blocked(OperationGateDecision::BlockedExcluded)
        );
        assert!(transformer.calls.is_empty());
        assert!(replacer.replacements.is_empty());
    }
}
