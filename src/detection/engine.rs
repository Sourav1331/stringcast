use crate::commands::{CommandDefinition, CommandKind, CommandRegistry, DynamicCommand};

pub const DYNAMIC_DEBOUNCE_MS: u64 = 650;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionDecision {
    NoMatch,
    Matched(TriggerMatch),
    PendingDynamic(PendingDynamicTrigger),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerMatch {
    pub trigger_text: String,
    pub transform_input: String,
    pub command: CommandDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDynamicTrigger {
    pub trigger_text: String,
    pub transform_input: String,
    pub debounce_ms: u64,
}

pub fn detect_trigger(buffer: &str, registry: &CommandRegistry) -> DetectionDecision {
    if let Some(static_match) = detect_static(buffer, registry) {
        return DetectionDecision::Matched(static_match);
    }

    detect_dynamic(buffer, false)
}

pub fn finalize_pending_dynamic(buffer: &str) -> DetectionDecision {
    detect_dynamic(buffer, true)
}

fn detect_static(buffer: &str, registry: &CommandRegistry) -> Option<TriggerMatch> {
    let trimmed_buffer = buffer.trim_end();
    for trigger in registry.static_triggers_longest_first() {
        if !ends_with_trigger(trimmed_buffer, &trigger) {
            continue;
        }

        let content_end = trimmed_buffer.len() - trigger.len();
        let transform_input = trimmed_buffer[..content_end].trim_end().to_string();

        let command = registry.resolve_static(&trigger)?;
        return Some(TriggerMatch {
            trigger_text: trigger,
            transform_input,
            command,
        });
    }

    None
}

fn detect_dynamic(buffer: &str, finalize: bool) -> DetectionDecision {
    let Some((prefix, prefix_start)) = last_unescaped_dynamic_prefix(buffer) else {
        return DetectionDecision::NoMatch;
    };

    let transform_input = buffer[..prefix_start].trim_end().to_string();

    let parameter = &buffer[prefix_start + prefix.len()..];
    match prefix {
        "?translate:" => detect_translate(parameter, transform_input, finalize),
        "?ask:" => detect_ask(parameter, transform_input, finalize),
        _ => DetectionDecision::NoMatch,
    }
}

fn detect_translate(parameter: &str, transform_input: String, finalize: bool) -> DetectionDecision {
    let finalizing_with_space = parameter.ends_with(' ');
    let lang_code = parameter.trim_end();
    if !is_valid_bcp47_subset(lang_code) {
        return DetectionDecision::NoMatch;
    }

    let trigger_text = format!("?translate:{lang_code}");
    if finalize || finalizing_with_space {
        return DetectionDecision::Matched(TriggerMatch {
            trigger_text,
            transform_input,
            command: dynamic_command(
                "?translate:",
                "Translate",
                DynamicCommand::Translate {
                    lang_code: lang_code.to_string(),
                },
            ),
        });
    }

    DetectionDecision::PendingDynamic(PendingDynamicTrigger {
        trigger_text,
        transform_input,
        debounce_ms: DYNAMIC_DEBOUNCE_MS,
    })
}

fn detect_ask(parameter: &str, transform_input: String, finalize: bool) -> DetectionDecision {
    let question = parameter.trim();
    if !(3..=256).contains(&question.chars().count()) {
        return DetectionDecision::NoMatch;
    }

    let trigger_text = format!("?ask:{question}");
    if finalize {
        return DetectionDecision::Matched(TriggerMatch {
            trigger_text,
            transform_input,
            command: dynamic_command(
                "?ask:",
                "Ask",
                DynamicCommand::Ask {
                    question: question.to_string(),
                },
            ),
        });
    }

    DetectionDecision::PendingDynamic(PendingDynamicTrigger {
        trigger_text,
        transform_input,
        debounce_ms: DYNAMIC_DEBOUNCE_MS,
    })
}

fn dynamic_command(trigger: &str, name: &str, command: DynamicCommand) -> CommandDefinition {
    CommandDefinition {
        trigger: trigger.to_string(),
        name: name.to_string(),
        prompt: String::new(),
        enabled: true,
        case_sensitive: true,
        raw_output: false,
        kind: CommandKind::Dynamic(command),
    }
}

fn ends_with_trigger(buffer: &str, trigger: &str) -> bool {
    buffer.ends_with(trigger) && !is_escaped_at(buffer, buffer.len() - trigger.len())
}

fn last_unescaped_dynamic_prefix(buffer: &str) -> Option<(&'static str, usize)> {
    ["?translate:", "?ask:"]
        .into_iter()
        .filter_map(|prefix| {
            buffer
                .match_indices(prefix)
                .filter(|(index, _)| !is_escaped_at(buffer, *index))
                .last()
                .map(|(index, _)| (prefix, index))
        })
        .max_by_key(|(_, index)| *index)
}

fn is_escaped_at(buffer: &str, index: usize) -> bool {
    if index == 0 {
        return false;
    }

    let preceding_backslashes = buffer[..index]
        .chars()
        .rev()
        .take_while(|ch| *ch == '\\')
        .count();

    preceding_backslashes % 2 == 1
}

fn is_valid_bcp47_subset(code: &str) -> bool {
    let mut parts = code.split('-');
    let Some(first) = parts.next() else {
        return false;
    };

    if !(2..=8).contains(&first.len()) || !first.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }

    parts.all(|part| {
        (2..=8).contains(&part.len()) && part.chars().all(|ch| ch.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{BuiltInCommand, CommandKind};

    #[test]
    fn detects_static_builtin_suffix() {
        let registry = CommandRegistry::new();
        let decision = detect_trigger("i dont no ?fix", &registry);

        let DetectionDecision::Matched(matched) = decision else {
            panic!("expected static match");
        };

        assert_eq!(matched.trigger_text, "?fix");
        assert_eq!(matched.transform_input, "i dont no");
        assert_eq!(
            matched.command.kind,
            CommandKind::BuiltIn(BuiltInCommand::Fix)
        );
    }

    #[test]
    fn detects_static_trigger_without_recent_buffer_prefix() {
        let registry = CommandRegistry::new();
        let decision = detect_trigger(" ?casual", &registry);

        let DetectionDecision::Matched(matched) = decision else {
            panic!("expected static match");
        };

        assert_eq!(matched.trigger_text, "?casual");
        assert_eq!(matched.transform_input, "");
        assert_eq!(
            matched.command.kind,
            CommandKind::BuiltIn(BuiltInCommand::Casual)
        );
    }

    #[test]
    fn detects_static_trigger_with_trailing_space() {
        let registry = CommandRegistry::new();
        let decision = detect_trigger("existing text ?formal ", &registry);

        let DetectionDecision::Matched(matched) = decision else {
            panic!("expected static match");
        };

        assert_eq!(matched.trigger_text, "?formal");
        assert_eq!(matched.transform_input, "existing text");
        assert_eq!(
            matched.command.kind,
            CommandKind::BuiltIn(BuiltInCommand::Formal)
        );
    }

    #[test]
    fn detects_every_builtin_without_recent_buffer_prefix() {
        let registry = CommandRegistry::new();
        let triggers = [
            "?fix",
            "?improve",
            "?shorten",
            "?expand",
            "?formal",
            "?casual",
            "?emoji",
            "?reply",
            "?bullets",
            "?summarize",
        ];

        for trigger in triggers {
            let decision = detect_trigger(&format!(" {trigger}"), &registry);
            assert!(
                matches!(decision, DetectionDecision::Matched(_)),
                "expected {trigger} to match"
            );
        }
    }

    #[test]
    fn escaped_static_trigger_does_not_match() {
        let registry = CommandRegistry::new();
        let decision = detect_trigger(r"literal \?fix", &registry);

        assert_eq!(decision, DetectionDecision::NoMatch);
    }

    #[test]
    fn translate_waits_for_debounce_when_still_typing() {
        let registry = CommandRegistry::new();
        let decision = detect_trigger("hello ?translate:es", &registry);

        assert!(matches!(decision, DetectionDecision::PendingDynamic(_)));
    }

    #[test]
    fn translate_trailing_space_finalizes() {
        let registry = CommandRegistry::new();
        let decision = detect_trigger("hello ?translate:es ", &registry);

        let DetectionDecision::Matched(matched) = decision else {
            panic!("expected dynamic match");
        };

        assert_eq!(matched.trigger_text, "?translate:es");
        assert_eq!(
            matched.command.kind,
            CommandKind::Dynamic(DynamicCommand::Translate {
                lang_code: "es".to_string()
            })
        );
    }

    #[test]
    fn translate_can_wait_without_recent_buffer_prefix() {
        let registry = CommandRegistry::new();
        let decision = detect_trigger(" ?translate:hi", &registry);

        assert!(matches!(decision, DetectionDecision::PendingDynamic(_)));
    }

    #[test]
    fn finalized_ask_can_match_without_recent_buffer_prefix() {
        let decision = finalize_pending_dynamic(" ?ask:make it casual");

        let DetectionDecision::Matched(matched) = decision else {
            panic!("expected finalized ask match");
        };

        assert_eq!(matched.transform_input, "");
        assert_eq!(
            matched.command.kind,
            CommandKind::Dynamic(DynamicCommand::Ask {
                question: "make it casual".to_string()
            })
        );
    }

    #[test]
    fn ask_spaces_remain_part_of_question_until_debounce() {
        let registry = CommandRegistry::new();
        let decision = detect_trigger("report ?ask:top three metrics", &registry);

        let DetectionDecision::PendingDynamic(pending) = decision else {
            panic!("expected pending ask trigger");
        };

        assert_eq!(pending.trigger_text, "?ask:top three metrics");
    }

    #[test]
    fn finalized_ask_produces_match() {
        let decision = finalize_pending_dynamic("report ?ask:top three metrics");

        let DetectionDecision::Matched(matched) = decision else {
            panic!("expected finalized ask trigger");
        };

        assert_eq!(
            matched.command.kind,
            CommandKind::Dynamic(DynamicCommand::Ask {
                question: "top three metrics".to_string()
            })
        );
    }
}
