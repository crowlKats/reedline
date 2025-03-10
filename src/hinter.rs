use crate::completion::HistoryCompleter;

use {
    crate::{Completer, History},
    nu_ansi_term::{Color, Style},
};

/// A trait that's responsible for returning the hint for the current line and position
/// Hints are often shown in-line as part of the buffer, showing the user text they can accept or ignore
pub trait Hinter {
    /// Handle the hinting duty by using the line, position, and current history
    fn handle(
        &mut self,
        line: &str,
        pos: usize,
        history: &dyn History,
        use_ansi_coloring: bool,
    ) -> String;

    /// Return the current hint being shown to the user
    fn current_hint(&self) -> String;
}

/// A default example hinter that use the completions or the history to show a hint to the user
pub struct DefaultHinter {
    completer: Option<Box<dyn Completer>>,
    history: bool,
    style: Style,
    inside_line: bool,
    current_hint: String,
}

impl Hinter for DefaultHinter {
    fn handle(
        &mut self,
        line: &str,
        pos: usize,
        history: &dyn History,
        use_ansi_coloring: bool,
    ) -> String {
        let mut completions = vec![];
        let mut output = String::new();

        if pos == line.len() || self.inside_line {
            if let Some(c) = &self.completer {
                completions = c.complete(line, pos);
            } else if self.history {
                let history: Vec<String> = history.iter_chronologic().cloned().collect();
                completions = HistoryCompleter::new(history).complete(line, pos);
            }

            if !completions.is_empty() {
                let mut hint = completions[0].1.clone();
                let span = completions[0].0;
                hint.replace_range(0..(span.end - span.start), "");

                let hint = hint.replace("\n", "\r\n");

                self.current_hint = hint.clone();

                output = self.style.paint(hint).to_string();
            } else {
                self.current_hint = String::new();
            }
        } else {
            self.current_hint = String::new();
        }

        if use_ansi_coloring {
            output
        } else if let Ok(bytes) = strip_ansi_escapes::strip(&output) {
            String::from_utf8_lossy(&bytes).to_string()
        } else {
            output
        }
    }

    fn current_hint(&self) -> String {
        self.current_hint.clone()
    }
}

impl Default for DefaultHinter {
    fn default() -> Self {
        DefaultHinter {
            completer: None,
            history: false,
            style: Style::new().fg(Color::LightGray),
            inside_line: false,
            current_hint: String::new(),
        }
    }
}

impl DefaultHinter {
    /// A builder for the default hinter that configures if the hint is shown inside the current line
    pub fn with_inside_line(mut self) -> DefaultHinter {
        self.inside_line = true;
        self
    }

    /// A builder that will configure the completer used by this hinter
    pub fn with_completer(mut self, completer: Box<dyn Completer>) -> DefaultHinter {
        self.completer = Some(completer);
        self
    }

    /// A builder that configures the history the hinter will use to hint, if in history mode
    pub fn with_history(mut self) -> DefaultHinter {
        self.history = true;
        self
    }

    /// A builder that sets the style applied to the hint as part of the buffer
    pub fn with_style(mut self, style: Style) -> DefaultHinter {
        self.style = style;
        self
    }
}
