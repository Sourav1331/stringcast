use super::SyntheticInputGuard;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSimulationError {
    Unavailable,
    Blocked,
}

pub trait InputSimulator {
    fn select_all(&mut self) -> Result<(), InputSimulationError>;
    fn copy(&mut self) -> Result<(), InputSimulationError>;
    fn paste(&mut self) -> Result<(), InputSimulationError>;
    fn type_text(&mut self, text: &str) -> Result<(), InputSimulationError>;
    fn collapse_selection(&mut self) -> Result<(), InputSimulationError>;
}

#[derive(Debug)]
pub struct GuardedInputSimulator<I> {
    inner: I,
    guard: SyntheticInputGuard,
}

impl<I> GuardedInputSimulator<I> {
    pub fn new(inner: I, guard: SyntheticInputGuard) -> Self {
        Self { inner, guard }
    }

    pub fn into_inner(self) -> I {
        self.inner
    }
}

impl<I> InputSimulator for GuardedInputSimulator<I>
where
    I: InputSimulator,
{
    fn select_all(&mut self) -> Result<(), InputSimulationError> {
        let _token = self.guard.acquire(Instant::now());
        self.inner.select_all()
    }

    fn copy(&mut self) -> Result<(), InputSimulationError> {
        let _token = self.guard.acquire(Instant::now());
        self.inner.copy()
    }

    fn paste(&mut self) -> Result<(), InputSimulationError> {
        let _token = self.guard.acquire(Instant::now());
        self.inner.paste()
    }

    fn type_text(&mut self, text: &str) -> Result<(), InputSimulationError> {
        let _token = self.guard.acquire(Instant::now());
        self.inner.type_text(text)
    }

    fn collapse_selection(&mut self) -> Result<(), InputSimulationError> {
        let _token = self.guard.acquire(Instant::now());
        self.inner.collapse_selection()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedInputAction {
    SelectAll,
    Copy,
    Paste,
    TypeText(String),
    CollapseSelection,
}

#[derive(Debug, Clone, Default)]
pub struct RecordingInputSimulator {
    pub actions: Vec<RecordedInputAction>,
}

impl InputSimulator for RecordingInputSimulator {
    fn select_all(&mut self) -> Result<(), InputSimulationError> {
        self.actions.push(RecordedInputAction::SelectAll);
        Ok(())
    }

    fn copy(&mut self) -> Result<(), InputSimulationError> {
        self.actions.push(RecordedInputAction::Copy);
        Ok(())
    }

    fn paste(&mut self) -> Result<(), InputSimulationError> {
        self.actions.push(RecordedInputAction::Paste);
        Ok(())
    }

    fn type_text(&mut self, text: &str) -> Result<(), InputSimulationError> {
        self.actions
            .push(RecordedInputAction::TypeText(text.to_string()));
        Ok(())
    }

    fn collapse_selection(&mut self) -> Result<(), InputSimulationError> {
        self.actions.push(RecordedInputAction::CollapseSelection);
        Ok(())
    }
}

pub struct EnigoInputSimulator {
    enigo: enigo::Enigo,
}

impl EnigoInputSimulator {
    pub fn new() -> Result<Self, InputSimulationError> {
        Ok(Self {
            enigo: enigo::Enigo::new(&enigo::Settings::default())
                .map_err(|_| InputSimulationError::Unavailable)?,
        })
    }

    fn key_combo(
        &mut self,
        modifier: enigo::Key,
        key: enigo::Key,
    ) -> Result<(), InputSimulationError> {
        use enigo::{Direction, Keyboard};

        self.enigo
            .key(modifier, Direction::Press)
            .map_err(|_| InputSimulationError::Unavailable)?;
        self.enigo
            .key(key, Direction::Click)
            .map_err(|_| InputSimulationError::Unavailable)?;
        self.enigo
            .key(modifier, Direction::Release)
            .map_err(|_| InputSimulationError::Unavailable)?;
        Ok(())
    }
}

impl InputSimulator for EnigoInputSimulator {
    fn select_all(&mut self) -> Result<(), InputSimulationError> {
        self.key_combo(primary_modifier(), enigo::Key::Unicode('a'))
    }

    fn copy(&mut self) -> Result<(), InputSimulationError> {
        self.key_combo(primary_modifier(), enigo::Key::Unicode('c'))
    }

    fn paste(&mut self) -> Result<(), InputSimulationError> {
        self.key_combo(primary_modifier(), enigo::Key::Unicode('v'))
    }

    fn type_text(&mut self, text: &str) -> Result<(), InputSimulationError> {
        use enigo::Keyboard;

        self.enigo
            .text(text)
            .map_err(|_| InputSimulationError::Unavailable)
    }

    fn collapse_selection(&mut self) -> Result<(), InputSimulationError> {
        use enigo::{Direction, Keyboard};

        self.enigo
            .key(enigo::Key::RightArrow, Direction::Click)
            .map_err(|_| InputSimulationError::Unavailable)
    }
}

#[cfg(target_os = "macos")]
fn primary_modifier() -> enigo::Key {
    enigo::Key::Meta
}

#[cfg(not(target_os = "macos"))]
fn primary_modifier() -> enigo::Key {
    enigo::Key::Control
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn guarded_simulator_suppresses_generated_actions() {
        let guard = SyntheticInputGuard::new(Duration::from_millis(250), Duration::from_secs(10));
        let mut simulator =
            GuardedInputSimulator::new(RecordingInputSimulator::default(), guard.clone());

        simulator.select_all().unwrap();

        assert!(guard.is_suppressed(Instant::now()));
        assert_eq!(
            simulator.into_inner().actions,
            vec![RecordedInputAction::SelectAll]
        );
    }
}
