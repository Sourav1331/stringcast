use crate::clipboard::{ClipboardBackend, ClipboardError};
use crate::detection::TriggerMatch;
use crate::input::{InputSimulationError, InputSimulator};
use crate::orchestrator::OperationSnapshot;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionContext {
    pub operation_id: u64,
    pub app_id: String,
    pub window_id: Option<String>,
    pub trigger_match: TriggerMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractionError {
    ClipboardUnavailable,
    CopyFailed,
    TriggerMissingFromSnapshot,
    AppBlocked,
}

pub trait TextExtractor {
    fn extract(&mut self, context: ExtractionContext)
        -> Result<OperationSnapshot, ExtractionError>;
}

#[derive(Debug, Clone, Default)]
pub struct BufferTextExtractor;

impl TextExtractor for BufferTextExtractor {
    fn extract(
        &mut self,
        context: ExtractionContext,
    ) -> Result<OperationSnapshot, ExtractionError> {
        Ok(OperationSnapshot {
            operation_id: context.operation_id,
            app_id: context.app_id,
            window_id: context.window_id,
            extracted_text: format!(
                "{} {}",
                context.trigger_match.transform_input, context.trigger_match.trigger_text
            ),
            replacement_target_text: format!(
                "{} {}",
                context.trigger_match.transform_input, context.trigger_match.trigger_text
            ),
            transform_input: context.trigger_match.transform_input,
            trigger_text: context.trigger_match.trigger_text,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ClipboardTextExtractor<C, I> {
    clipboard: C,
    input: I,
    select_all_wait: Duration,
    clipboard_read_wait: Duration,
}

impl<C, I> ClipboardTextExtractor<C, I> {
    pub fn new(clipboard: C, input: I) -> Self {
        Self::with_delays(clipboard, input, Duration::ZERO, Duration::ZERO)
    }

    pub fn with_delays(
        clipboard: C,
        input: I,
        select_all_wait: Duration,
        clipboard_read_wait: Duration,
    ) -> Self {
        Self {
            clipboard,
            input,
            select_all_wait,
            clipboard_read_wait,
        }
    }

    pub fn into_parts(self) -> (C, I) {
        (self.clipboard, self.input)
    }
}

impl<C, I> TextExtractor for ClipboardTextExtractor<C, I>
where
    C: ClipboardBackend,
    I: InputSimulator,
{
    fn extract(
        &mut self,
        context: ExtractionContext,
    ) -> Result<OperationSnapshot, ExtractionError> {
        let original_clipboard = self.clipboard.snapshot()?;

        thread::sleep(self.select_all_wait);
        if let Err(error) = self.input.select_all() {
            self.cleanup_failed_extraction(&original_clipboard);
            return Err(error.into());
        }
        thread::sleep(self.select_all_wait);
        if let Err(error) = self.input.copy() {
            self.cleanup_failed_extraction(&original_clipboard);
            return Err(error.into());
        }
        thread::sleep(self.clipboard_read_wait);

        let copied_text = match self.clipboard.get_text() {
            Ok(Some(text)) => text,
            Ok(None) => {
                self.cleanup_failed_extraction(&original_clipboard);
                return Err(ExtractionError::TriggerMissingFromSnapshot);
            }
            Err(error) => {
                self.cleanup_failed_extraction(&original_clipboard);
                return Err(error.into());
            }
        };

        let transform_input = match transform_input_from_snapshot(
            &copied_text,
            &context.trigger_match.trigger_text,
        ) {
            Ok(transform_input) => transform_input,
            Err(error) => {
                self.cleanup_failed_extraction(&original_clipboard);
                return Err(error);
            }
        };

        if let Err(error) = self.clipboard.restore(&original_clipboard) {
            self.cleanup_failed_extraction(&original_clipboard);
            return Err(error.into());
        }

        Ok(OperationSnapshot {
            operation_id: context.operation_id,
            app_id: context.app_id,
            window_id: context.window_id,
            replacement_target_text: copied_text.clone(),
            extracted_text: copied_text,
            transform_input,
            trigger_text: context.trigger_match.trigger_text,
        })
    }
}

impl<C, I> ClipboardTextExtractor<C, I>
where
    C: ClipboardBackend,
    I: InputSimulator,
{
    fn cleanup_failed_extraction(
        &mut self,
        original_clipboard: &crate::clipboard::ClipboardSnapshot,
    ) {
        let _ = self.input.collapse_selection();
        let _ = self.clipboard.restore(original_clipboard);
    }
}

fn transform_input_from_snapshot(
    copied_text: &str,
    trigger_text: &str,
) -> Result<String, ExtractionError> {
    let trimmed = copied_text.trim_end();
    if !trimmed.ends_with(trigger_text) {
        return Err(ExtractionError::TriggerMissingFromSnapshot);
    }

    let input_end = trimmed.len() - trigger_text.len();
    Ok(trimmed[..input_end].trim_end().to_string())
}

impl From<ClipboardError> for ExtractionError {
    fn from(_error: ClipboardError) -> Self {
        Self::ClipboardUnavailable
    }
}

impl From<InputSimulationError> for ExtractionError {
    fn from(_error: InputSimulationError) -> Self {
        Self::CopyFailed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{ClipboardBackend, ClipboardSnapshot, MemoryClipboard};
    use crate::commands::{BuiltInCommand, CommandDefinition, CommandKind};
    use crate::input::{RecordedInputAction, RecordingInputSimulator};

    #[derive(Debug, Clone)]
    struct RestoreFailingClipboard {
        text: Option<String>,
    }

    impl ClipboardBackend for RestoreFailingClipboard {
        fn snapshot(&mut self) -> Result<ClipboardSnapshot, ClipboardError> {
            Ok(ClipboardSnapshot {
                text: self.text.clone(),
            })
        }

        fn get_text(&mut self) -> Result<Option<String>, ClipboardError> {
            Ok(self.text.clone())
        }

        fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
            self.text = Some(text.to_string());
            Ok(())
        }

        fn restore(&mut self, _snapshot: &ClipboardSnapshot) -> Result<(), ClipboardError> {
            Err(ClipboardError::Unavailable)
        }
    }

    fn context() -> ExtractionContext {
        ExtractionContext {
            operation_id: 7,
            app_id: "com.example.App".to_string(),
            window_id: Some("window".to_string()),
            trigger_match: TriggerMatch {
                trigger_text: "?fix".to_string(),
                transform_input: "buffer fallback".to_string(),
                command: CommandDefinition {
                    trigger: "?fix".to_string(),
                    name: "Fix".to_string(),
                    prompt: "Fix: {text}".to_string(),
                    enabled: true,
                    case_sensitive: true,
                    raw_output: false,
                    kind: CommandKind::BuiltIn(BuiltInCommand::Fix),
                },
            },
        }
    }

    #[test]
    fn clipboard_extractor_uses_copied_field_snapshot() {
        let clipboard = MemoryClipboard::new(Some("actual field text ?fix".to_string()));
        let input = RecordingInputSimulator::default();
        let mut extractor = ClipboardTextExtractor::new(clipboard, input);

        let snapshot = extractor.extract(context()).unwrap();
        let (_, input) = extractor.into_parts();

        assert_eq!(snapshot.extracted_text, "actual field text ?fix");
        assert_eq!(snapshot.replacement_target_text, "actual field text ?fix");
        assert_eq!(snapshot.transform_input, "actual field text");
        assert_eq!(
            input.actions,
            vec![RecordedInputAction::SelectAll, RecordedInputAction::Copy]
        );
    }

    #[test]
    fn clipboard_extractor_rejects_snapshot_without_trigger() {
        let clipboard = MemoryClipboard::new(Some("actual field text".to_string()));
        let input = RecordingInputSimulator::default();
        let mut extractor = ClipboardTextExtractor::new(clipboard, input);

        let result = extractor.extract(context());
        let (_, input) = extractor.into_parts();

        assert_eq!(result, Err(ExtractionError::TriggerMissingFromSnapshot));
        assert_eq!(
            input.actions,
            vec![
                RecordedInputAction::SelectAll,
                RecordedInputAction::Copy,
                RecordedInputAction::CollapseSelection
            ]
        );
    }

    #[test]
    fn clipboard_extractor_accepts_trigger_without_input_text() {
        let clipboard = MemoryClipboard::new(Some("?fix".to_string()));
        let input = RecordingInputSimulator::default();
        let mut extractor = ClipboardTextExtractor::new(clipboard, input);

        let snapshot = extractor.extract(context()).unwrap();

        assert_eq!(snapshot.extracted_text, "?fix");
        assert_eq!(snapshot.replacement_target_text, "?fix");
        assert_eq!(snapshot.transform_input, "");
    }

    #[test]
    fn clipboard_extractor_collapses_selection_when_restore_fails() {
        let clipboard = RestoreFailingClipboard {
            text: Some("actual field text ?fix".to_string()),
        };
        let input = RecordingInputSimulator::default();
        let mut extractor = ClipboardTextExtractor::new(clipboard, input);

        let result = extractor.extract(context());
        let (_, input) = extractor.into_parts();

        assert_eq!(result, Err(ExtractionError::ClipboardUnavailable));
        assert_eq!(
            input.actions,
            vec![
                RecordedInputAction::SelectAll,
                RecordedInputAction::Copy,
                RecordedInputAction::CollapseSelection
            ]
        );
    }
}
