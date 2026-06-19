use super::{InputEvent, KeystrokeBuffer, SyntheticInputGuard};
use crate::detection::DYNAMIC_DEBOUNCE_MS;
use crate::extraction::TextExtractor;
use crate::pipeline::{PipelineError, PipelineOutcome, TextTransformer, TransformationPipeline};
use crate::platform::{ForegroundAppProvider, OperationGate};
use crate::replacement::TextReplacer;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputControllerOutcome {
    IgnoredSynthetic,
    BufferUpdated(String),
    BufferCleared,
    Pipeline(PipelineOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputControllerError {
    Pipeline(PipelineError),
}

impl From<PipelineError> for InputControllerError {
    fn from(error: PipelineError) -> Self {
        Self::Pipeline(error)
    }
}

pub struct InputController<E, T, R, P> {
    buffer: KeystrokeBuffer,
    pipeline: TransformationPipeline<E, T, R>,
    foreground_provider: P,
    gate: OperationGate,
    synthetic_guard: SyntheticInputGuard,
    pending_dynamic_deadline: Option<Instant>,
}

impl<E, T, R, P> InputController<E, T, R, P>
where
    E: TextExtractor,
    T: TextTransformer,
    R: TextReplacer,
    P: ForegroundAppProvider,
{
    pub fn new(
        pipeline: TransformationPipeline<E, T, R>,
        foreground_provider: P,
        gate: OperationGate,
        synthetic_guard: SyntheticInputGuard,
    ) -> Self {
        Self {
            buffer: KeystrokeBuffer::default(),
            pipeline,
            foreground_provider,
            gate,
            synthetic_guard,
            pending_dynamic_deadline: None,
        }
    }

    pub fn handle_event(
        &mut self,
        event: InputEvent,
        now: Instant,
    ) -> Result<InputControllerOutcome, InputControllerError> {
        if self.synthetic_guard.is_suppressed(now) {
            return Ok(InputControllerOutcome::IgnoredSynthetic);
        }

        match event {
            InputEvent::Text(text) => {
                self.buffer.append(&text);
                let outcome = self.pipeline.process_foreground_buffer(
                    self.buffer.as_str(),
                    &mut self.foreground_provider,
                    &self.gate,
                )?;
                match outcome {
                    PipelineOutcome::NoMatch => {
                        self.pending_dynamic_deadline = None;
                        Ok(InputControllerOutcome::BufferUpdated(
                            self.buffer.as_str().to_string(),
                        ))
                    }
                    PipelineOutcome::PendingDynamic => {
                        self.pending_dynamic_deadline =
                            Some(now + Duration::from_millis(DYNAMIC_DEBOUNCE_MS));
                        Ok(InputControllerOutcome::Pipeline(
                            PipelineOutcome::PendingDynamic,
                        ))
                    }
                    PipelineOutcome::Blocked(decision) => {
                        self.clear_buffer_and_pending();
                        Ok(InputControllerOutcome::Pipeline(PipelineOutcome::Blocked(
                            decision,
                        )))
                    }
                    PipelineOutcome::Replaced { .. } => {
                        self.clear_buffer_and_pending();
                        Ok(InputControllerOutcome::Pipeline(outcome))
                    }
                }
            }
            InputEvent::Backspace => {
                let rearm_pending_dynamic = self.pending_dynamic_deadline.is_some();
                self.buffer.backspace();
                if rearm_pending_dynamic {
                    self.pending_dynamic_deadline =
                        Some(now + Duration::from_millis(DYNAMIC_DEBOUNCE_MS));
                }
                Ok(InputControllerOutcome::BufferUpdated(
                    self.buffer.as_str().to_string(),
                ))
            }
            InputEvent::Delete
            | InputEvent::Enter
            | InputEvent::Escape
            | InputEvent::Tab
            | InputEvent::Navigation(_)
            | InputEvent::MouseButton
            | InputEvent::Shortcut(_)
            | InputEvent::FocusChanged
            | InputEvent::SleepOrLock => {
                self.clear_buffer_and_pending();
                Ok(InputControllerOutcome::BufferCleared)
            }
        }
    }

    pub fn handle_pending_timeout(
        &mut self,
        now: Instant,
    ) -> Result<InputControllerOutcome, InputControllerError> {
        let Some(deadline) = self.pending_dynamic_deadline else {
            return Ok(InputControllerOutcome::BufferUpdated(
                self.buffer.as_str().to_string(),
            ));
        };

        if now < deadline {
            return Ok(InputControllerOutcome::BufferUpdated(
                self.buffer.as_str().to_string(),
            ));
        }

        self.pending_dynamic_deadline = None;
        let outcome = self.pipeline.finalize_pending_foreground_buffer(
            self.buffer.as_str(),
            &mut self.foreground_provider,
            &self.gate,
        )?;

        match outcome {
            PipelineOutcome::NoMatch | PipelineOutcome::PendingDynamic => Ok(
                InputControllerOutcome::BufferUpdated(self.buffer.as_str().to_string()),
            ),
            PipelineOutcome::Blocked(decision) => {
                self.clear_buffer_and_pending();
                Ok(InputControllerOutcome::Pipeline(PipelineOutcome::Blocked(
                    decision,
                )))
            }
            PipelineOutcome::Replaced { .. } => {
                self.clear_buffer_and_pending();
                Ok(InputControllerOutcome::Pipeline(outcome))
            }
        }
    }

    pub fn pending_dynamic_deadline(&self) -> Option<Instant> {
        self.pending_dynamic_deadline
    }

    pub fn buffer(&self) -> &str {
        self.buffer.as_str()
    }

    pub fn into_parts(self) -> (TransformationPipeline<E, T, R>, P) {
        (self.pipeline, self.foreground_provider)
    }

    fn clear_buffer_and_pending(&mut self) {
        self.buffer.clear();
        self.pending_dynamic_deadline = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandDefinition, CommandRegistry};
    use crate::detection::DYNAMIC_DEBOUNCE_MS;
    use crate::extraction::BufferTextExtractor;
    use crate::platform::{
        ExclusionMatcher, ForegroundApp, OperationGate, StaticForegroundAppProvider,
    };
    use crate::replacement::NoopTextReplacer;
    use std::time::Duration;

    #[derive(Debug, Clone)]
    struct FakeTransformer;

    impl TextTransformer for FakeTransformer {
        fn transform(
            &mut self,
            _command: &CommandDefinition,
            input: &str,
        ) -> Result<String, crate::pipeline::TransformError> {
            Ok(format!("fixed: {input}"))
        }
    }

    fn controller() -> InputController<
        BufferTextExtractor,
        FakeTransformer,
        NoopTextReplacer,
        StaticForegroundAppProvider,
    > {
        let pipeline = TransformationPipeline::new(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer,
            NoopTextReplacer::default(),
        );
        let foreground_provider = StaticForegroundAppProvider::new(ForegroundApp {
            app_id: "com.example.App".to_string(),
            window_id: None,
            display_name: None,
            secure_input: false,
            elevated: false,
        });
        let gate = OperationGate::new(true, ExclusionMatcher::new(Vec::new()));
        InputController::new(
            pipeline,
            foreground_provider,
            gate,
            SyntheticInputGuard::new(Duration::from_millis(250), Duration::from_secs(10)),
        )
    }

    #[test]
    fn text_events_update_buffer_until_trigger_matches() {
        let now = Instant::now();
        let mut controller = controller();

        let first = controller
            .handle_event(InputEvent::Text("hello ".to_string()), now)
            .unwrap();
        let second = controller
            .handle_event(InputEvent::Text("?fix".to_string()), now)
            .unwrap();

        assert_eq!(
            first,
            InputControllerOutcome::BufferUpdated("hello ".to_string())
        );
        assert!(matches!(
            second,
            InputControllerOutcome::Pipeline(PipelineOutcome::Replaced { .. })
        ));
        assert_eq!(controller.buffer(), "");
    }

    #[test]
    fn pending_dynamic_trigger_replaces_after_debounce() {
        let now = Instant::now();
        let mut controller = controller();

        let first = controller
            .handle_event(
                InputEvent::Text("hello ?ask:make this warmer".to_string()),
                now,
            )
            .unwrap();

        assert_eq!(
            first,
            InputControllerOutcome::Pipeline(PipelineOutcome::PendingDynamic)
        );
        assert!(controller.pending_dynamic_deadline().is_some());

        let outcome = controller
            .handle_pending_timeout(now + Duration::from_millis(DYNAMIC_DEBOUNCE_MS + 1))
            .unwrap();

        assert!(matches!(
            outcome,
            InputControllerOutcome::Pipeline(PipelineOutcome::Replaced { .. })
        ));
        assert_eq!(controller.buffer(), "");
    }

    #[test]
    fn backspace_extends_pending_dynamic_debounce() {
        let now = Instant::now();
        let mut controller = controller();

        controller
            .handle_event(
                InputEvent::Text("hello ?ask:make this warmer".to_string()),
                now,
            )
            .unwrap();
        let original_deadline = controller.pending_dynamic_deadline().unwrap();

        controller
            .handle_event(InputEvent::Backspace, now + Duration::from_millis(100))
            .unwrap();
        let updated_deadline = controller.pending_dynamic_deadline().unwrap();

        assert!(updated_deadline > original_deadline);

        let early = controller
            .handle_pending_timeout(original_deadline + Duration::from_millis(1))
            .unwrap();
        assert!(matches!(early, InputControllerOutcome::BufferUpdated(_)));

        let final_outcome = controller
            .handle_pending_timeout(updated_deadline + Duration::from_millis(1))
            .unwrap();
        assert!(matches!(
            final_outcome,
            InputControllerOutcome::Pipeline(PipelineOutcome::Replaced { .. })
        ));
    }

    #[test]
    fn navigation_clears_buffer() {
        let now = Instant::now();
        let mut controller = controller();

        controller
            .handle_event(InputEvent::Text("hello".to_string()), now)
            .unwrap();
        let outcome = controller
            .handle_event(
                InputEvent::Navigation(super::super::NavigationKey::ArrowLeft),
                now,
            )
            .unwrap();

        assert_eq!(outcome, InputControllerOutcome::BufferCleared);
        assert_eq!(controller.buffer(), "");
    }

    #[test]
    fn synthetic_events_are_ignored() {
        let guard = SyntheticInputGuard::new(Duration::from_millis(250), Duration::from_secs(10));
        let pipeline = TransformationPipeline::new(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer,
            NoopTextReplacer::default(),
        );
        let foreground_provider = StaticForegroundAppProvider::new(ForegroundApp {
            app_id: "com.example.App".to_string(),
            window_id: None,
            display_name: None,
            secure_input: false,
            elevated: false,
        });
        let gate = OperationGate::new(true, ExclusionMatcher::new(Vec::new()));
        let now = Instant::now();
        let _token = guard.acquire(now);
        let mut controller = InputController::new(pipeline, foreground_provider, gate, guard);

        let outcome = controller
            .handle_event(InputEvent::Text("?fix".to_string()), now)
            .unwrap();

        assert_eq!(outcome, InputControllerOutcome::IgnoredSynthetic);
        assert_eq!(controller.buffer(), "");
    }

    #[test]
    fn blocked_foreground_context_clears_buffer() {
        let pipeline = TransformationPipeline::new(
            CommandRegistry::new(),
            BufferTextExtractor,
            FakeTransformer,
            NoopTextReplacer::default(),
        );
        let foreground_provider = StaticForegroundAppProvider::new(ForegroundApp {
            app_id: "1Password.exe".to_string(),
            window_id: None,
            display_name: None,
            secure_input: false,
            elevated: false,
        });
        let gate = OperationGate::new(true, ExclusionMatcher::from_config(&Default::default()));
        let mut controller = InputController::new(
            pipeline,
            foreground_provider,
            gate,
            SyntheticInputGuard::default(),
        );

        let outcome = controller
            .handle_event(InputEvent::Text("secret ?fix".to_string()), Instant::now())
            .unwrap();

        assert!(matches!(
            outcome,
            InputControllerOutcome::Pipeline(PipelineOutcome::Blocked(_))
        ));
        assert_eq!(controller.buffer(), "");
    }
}
