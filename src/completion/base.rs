use crate::core_editor::LineBuffer;

/// A span of source code, with positions in bytes
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub struct Span {
    /// The starting position of the span, in bytes
    pub start: usize,

    /// The ending position of the span, in bytes
    pub end: usize,
}

impl Span {
    /// Creates a new `Span` from start and end inputs.
    /// The end parameter must be greater than or equal to the start parameter.
    ///
    /// # Panics
    /// If `end < start`
    pub fn new(start: usize, end: usize) -> Span {
        assert!(
            end >= start,
            "Can't create a Span whose end < start, start={}, end={}",
            start,
            end
        );

        Span { start, end }
    }
}

/// The handler for when the user begins a completion action, often using the tab key
/// This handler will then present the options to the user, allowing them to navigate the options
/// and pick the completion they want
pub trait CompletionActionHandler {
    /// Handle the completion action from the given line buffer
    fn handle(&mut self, line: &mut LineBuffer);
}

/// A trait that defines how to convert a line and position to a list of potential completions in that position.
pub trait Completer {
    /// the action that will take the line and position and convert it to a vector of completions, which include the
    /// span to replace and the contents of that replacement
    fn complete(&self, line: &str, pos: usize) -> Vec<(Span, String)>;
}
