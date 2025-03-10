use std::borrow::Borrow;

use {
    crate::{
        completion::{CircularCompletionHandler, CompletionActionHandler},
        core_editor::Editor,
        edit_mode::{EditMode, Emacs},
        enums::{ReedlineEvent, UndoBehavior},
        hinter::{DefaultHinter, Hinter},
        history::{FileBackedHistory, History, HistoryNavigationQuery},
        painter::Painter,
        prompt::{PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus},
        text_manipulation, DefaultHighlighter, DefaultValidator, EditCommand, Highlighter, Prompt,
        Signal, ValidationResult, Validator,
    },
    crossterm::{event, event::Event, terminal, Result},
    std::{io, time::Duration},
};

// These two parameters define when an event is a Paste Event. The POLL_WAIT is used
// to specify for how long the POLL should wait for events. Having a POLL_WAIT
// of zero means that every single event is treated as soon as it arrives. This
// doesn't allow for the possibility of more than 1 event happening at the same
// time.
const POLL_WAIT: u64 = 10;
// Since a paste event is multiple Event::Key events happening at the same time, we specify
// how many events should be in the crossterm_events vector before it is considered
// a paste. 10 events in 10 milliseconds is conservative enough (unlikely somebody
// will type more than 10 characters in 10 milliseconds)
const EVENTS_THRESHOLD: usize = 10;

/// Determines if inputs should be used to extend the regular line buffer,
/// traverse the history in the standard prompt or edit the search string in the
/// reverse search
#[derive(Debug, PartialEq, Eq)]
enum InputMode {
    /// Regular input by user typing or previous insertion.
    /// Undo tracking is active
    Regular,
    /// Full reverse search mode with different prompt,
    /// editing affects the search string,
    /// suggestions are provided to be inserted in the line buffer
    HistorySearch,
    /// Hybrid mode indicating that history is walked through in the standard prompt
    /// Either bash style up/down history or fish style prefix search,
    /// Edits directly switch to [`InputMode::Regular`]
    HistoryTraversal,
}

/// Line editor engine
///
/// ## Example usage
/// ```no_run
/// use std::io;
/// use reedline::{Reedline, Signal, DefaultPrompt};
/// let mut line_editor = Reedline::create()?;
/// let prompt = DefaultPrompt::default();
///
/// let out = line_editor.read_line(&prompt).unwrap();
/// match out {
///    Signal::Success(content) => {
///        // process content
///    }
///    _ => {
///        eprintln!("Entry aborted!");
///
///    }
/// }
/// # Ok::<(), io::Error>(())
/// ```
pub struct Reedline {
    editor: Editor,

    // History
    history: Box<dyn History>,
    input_mode: InputMode,

    // Validator
    validator: Box<dyn Validator>,

    // Stdout
    painter: Painter,

    // Edit Mode: Vi, Emacs
    edit_mode: Box<dyn EditMode>,

    // Perform action when user hits tab
    tab_handler: Box<dyn CompletionActionHandler>,

    // Highlight the edit buffer
    highlighter: Box<dyn Highlighter>,

    // Showcase hints based on various strategies (history, language-completion, spellcheck, etc)
    hinter: Box<dyn Hinter>,

    // Is Some(n) read_line() should repaint prompt every `n` milliseconds
    animate: bool,

    // Use ansi coloring or not
    use_ansi_coloring: bool,
}

impl Drop for Reedline {
    fn drop(&mut self) {
        // Ensures that the terminal is in a good state if we panic semigracefully
        // Calling `disable_raw_mode()` twice is fine with Linux
        let _ = terminal::disable_raw_mode();
    }
}

impl Reedline {
    /// Create a new [`Reedline`] engine with a local [`History`] that is not synchronized to a file.
    pub fn create() -> io::Result<Reedline> {
        let history = Box::new(FileBackedHistory::default());
        let painter = Painter::new(io::stdout());
        let buffer_highlighter = Box::new(DefaultHighlighter::default());
        let hinter = Box::new(DefaultHinter::default());
        let validator = Box::new(DefaultValidator);

        let edit_mode = Box::new(Emacs::default());

        let reedline = Reedline {
            editor: Editor::default(),
            history,
            input_mode: InputMode::Regular,
            painter,
            edit_mode,
            tab_handler: Box::new(CircularCompletionHandler::default()),
            highlighter: buffer_highlighter,
            hinter,
            validator,
            animate: true,
            use_ansi_coloring: true,
        };

        Ok(reedline)
    }

    /// A builder to include the hinter in your instance of the Reedline engine
    /// # Example
    /// ```rust,no_run
    /// //Cargo.toml
    /// //[dependencies]
    /// //nu-ansi-term = "*"
    /// use std::io;
    /// use {
    ///     nu_ansi_term::{Color, Style},
    ///     reedline::{DefaultCompleter, DefaultHinter, Reedline},
    /// };
    ///
    /// let commands = vec![
    ///     "test".into(),
    ///     "hello world".into(),
    ///     "hello world reedline".into(),
    ///     "this is the reedline crate".into(),
    /// ];
    /// let completer = Box::new(DefaultCompleter::new_with_wordlen(commands.clone(), 2));
    ///
    /// let mut line_editor = Reedline::create()?.with_hinter(Box::new(
    ///     DefaultHinter::default()
    ///     .with_completer(completer) // or .with_history()
    ///     // .with_inside_line()
    ///     .with_style(Style::new().italic().fg(Color::LightGray)),
    /// ));
    /// # Ok::<(), io::Error>(())
    /// ```
    pub fn with_hinter(mut self, hinter: Box<dyn Hinter>) -> Reedline {
        self.hinter = hinter;
        self
    }

    /// A builder to configure the completion action handler to use in your instance of the reedline engine
    /// # Example
    /// ```rust,no_run
    /// // Create a reedline object with tab completions support
    ///
    /// use std::io;
    /// use reedline::{DefaultCompleter, CircularCompletionHandler, Reedline};
    ///
    /// let commands = vec![
    ///   "test".into(),
    ///   "hello world".into(),
    ///   "hello world reedline".into(),
    ///   "this is the reedline crate".into(),
    /// ];
    /// let completer = Box::new(DefaultCompleter::new_with_wordlen(commands.clone(), 2));
    ///
    /// let mut line_editor = Reedline::create()?.with_completion_action_handler(Box::new(
    ///   CircularCompletionHandler::default().with_completer(completer),
    /// ));
    /// # Ok::<(), io::Error>(())
    /// ```
    pub fn with_completion_action_handler(
        mut self,
        tab_handler: Box<dyn CompletionActionHandler>,
    ) -> Reedline {
        self.tab_handler = tab_handler;
        self
    }

    /// A builder which enables or disables the use of ansi coloring in the prompt
    /// and in the command line syntax highlighting.
    pub fn with_ansi_colors(mut self, use_ansi_coloring: bool) -> Reedline {
        self.use_ansi_coloring = use_ansi_coloring;
        self
    }

    /// A builder which enables or disables animations/automatic repainting of prompt.
    /// If `repaint` is true, every second the prompt will be repainted and the clock updates
    pub fn with_animation(mut self, repaint: bool) -> Reedline {
        self.animate = repaint;
        self
    }

    /// A builder that configures the highlighter for your instance of the Reedline engine
    /// # Example
    /// ```rust,no_run
    /// // Create a reedline object with highlighter support
    ///
    /// use std::io;
    /// use reedline::{DefaultHighlighter, Reedline};
    ///
    /// let commands = vec![
    ///   "test".into(),
    ///   "hello world".into(),
    ///   "hello world reedline".into(),
    ///   "this is the reedline crate".into(),
    /// ];
    /// let mut line_editor =
    /// Reedline::create()?.with_highlighter(Box::new(DefaultHighlighter::new(commands)));
    /// # Ok::<(), io::Error>(())
    /// ```
    pub fn with_highlighter(mut self, highlighter: Box<dyn Highlighter>) -> Reedline {
        self.highlighter = highlighter;
        self
    }

    /// A builder which configures the history for your instance of the Reedline engine
    /// # Example
    /// ```rust,no_run
    /// // Create a reedline object with history support, including history size limits
    ///
    /// use std::io;
    /// use reedline::{FileBackedHistory, Reedline};
    ///
    /// let history = Box::new(
    /// FileBackedHistory::with_file(5, "history.txt".into())
    ///     .expect("Error configuring history with file"),
    /// );
    /// let mut line_editor = Reedline::create()?
    ///     .with_history(history)
    ///     .expect("Error configuring reedline with history");
    /// # Ok::<(), io::Error>(())
    /// ```
    pub fn with_history(mut self, history: Box<dyn History>) -> std::io::Result<Reedline> {
        self.history = history;

        Ok(self)
    }

    /// A builder that configures the validator for your instance of the Reedline engine
    /// # Example
    /// ```rust,no_run
    /// // Create a reedline object with validator support
    ///
    /// use std::io;
    /// use reedline::{DefaultValidator, Reedline};
    ///
    /// let mut line_editor =
    /// Reedline::create()?.with_validator(Box::new(DefaultValidator));
    /// # Ok::<(), io::Error>(())
    /// ```
    pub fn with_validator(mut self, validator: Box<dyn Validator>) -> Reedline {
        self.validator = validator;
        self
    }

    /// A builder which configures the edit mode for your instance of the Reedline engine
    pub fn with_edit_mode(mut self, edit_mode: Box<dyn EditMode>) -> Reedline {
        self.edit_mode = edit_mode;

        self
    }

    /// Returns the corresponding expected prompt style for the given edit mode
    pub fn prompt_edit_mode(&self) -> PromptEditMode {
        self.edit_mode.edit_mode()
    }

    /// Output the complete [`History`] chronologically with numbering to the terminal
    pub fn print_history(&mut self) -> Result<()> {
        let history: Vec<_> = self
            .history
            .iter_chronologic()
            .cloned()
            .enumerate()
            .collect();

        for (i, entry) in history {
            self.print_line(&format!("{}\t{}", i + 1, entry))?;
        }
        Ok(())
    }

    /// Wait for input and provide the user with a specified [`Prompt`].
    ///
    /// Returns a [`crossterm::Result`] in which the `Err` type is [`crossterm::ErrorKind`]
    /// to distinguish I/O errors and the `Ok` variant wraps a [`Signal`] which
    /// handles user inputs.
    pub fn read_line(&mut self, prompt: &dyn Prompt) -> Result<Signal> {
        terminal::enable_raw_mode()?;

        let result = self.read_line_helper(prompt);

        terminal::disable_raw_mode()?;

        result
    }

    /// Writes `msg` to the terminal with a following carriage return and newline
    pub fn print_line(&mut self, msg: &str) -> Result<()> {
        self.painter.paint_line(msg)
    }

    /// Clear the screen by printing enough whitespace to start the prompt or
    /// other output back at the first line of the terminal.
    pub fn clear_screen(&mut self) -> Result<()> {
        self.painter.clear_screen()?;

        Ok(())
    }

    /// Helper implemting the logic for [`Reedline::read_line()`] to be wrapped
    /// in a `raw_mode` context.
    fn read_line_helper(&mut self, prompt: &dyn Prompt) -> Result<Signal> {
        self.painter.init_terminal_size()?;
        self.painter.initialize_prompt_position()?;

        // Redraw if Ctrl-L was used
        if self.input_mode == InputMode::HistorySearch {
            self.history_search_paint(prompt)?;
        } else {
            self.full_repaint(prompt)?;
        }

        let mut crossterm_events: Vec<Event> = vec![];
        let mut reedline_events: Vec<ReedlineEvent> = vec![];

        loop {
            if event::poll(Duration::from_millis(1000))? {
                let mut latest_resize = None;

                // There could be multiple events queued up!
                // pasting text, resizes, blocking this thread (e.g. during debugging)
                // We should be able to handle all of them as quickly as possible without causing unnecessary output steps.
                while event::poll(Duration::from_millis(POLL_WAIT))? {
                    // TODO: Maybe replace with a separate function processing the buffered event
                    match event::read()? {
                        Event::Resize(x, y) => {
                            latest_resize = Some((x, y));
                        }
                        x => crossterm_events.push(x),
                    }
                }

                if let Some((x, y)) = latest_resize {
                    reedline_events.push(ReedlineEvent::Resize(x, y));
                }

                let mut last_edit_commands = None;
                // If the size of crossterm_event vector is larger than threshold, we could assume
                // that a lot of events were pasted into the prompt, indicating a paste
                if crossterm_events.len() > EVENTS_THRESHOLD {
                    reedline_events.push(self.handle_paste(&mut crossterm_events));
                } else {
                    for event in crossterm_events.drain(..) {
                        match (&mut last_edit_commands, self.edit_mode.parse_event(event)) {
                            (None, ReedlineEvent::Edit(ec)) => {
                                last_edit_commands = Some(ec);
                            }
                            (None, other_event) => {
                                reedline_events.push(other_event);
                            }
                            (Some(ref mut last_ecs), ReedlineEvent::Edit(ec)) => {
                                last_ecs.extend(ec);
                            }
                            (ref mut a @ Some(_), other_event) => {
                                reedline_events.push(ReedlineEvent::Edit(a.take().unwrap()));

                                reedline_events.push(other_event);
                            }
                        }
                    }
                }
                if let Some(ec) = last_edit_commands {
                    reedline_events.push(ReedlineEvent::Edit(ec));
                }
            } else if self.animate {
                reedline_events.push(ReedlineEvent::Repaint);
            };

            for event in reedline_events.drain(..) {
                if let Some(signal) = self.handle_event(prompt, event)? {
                    return Ok(signal);
                }
            }
        }
    }

    fn handle_paste(&mut self, crossterm_events: &mut Vec<Event>) -> ReedlineEvent {
        let reedline_events = crossterm_events
            .drain(..)
            .map(|event| self.edit_mode.parse_event(event))
            .collect::<Vec<ReedlineEvent>>();

        ReedlineEvent::Paste(reedline_events)
    }

    fn handle_event(
        &mut self,
        prompt: &dyn Prompt,
        event: ReedlineEvent,
    ) -> Result<Option<Signal>> {
        if self.input_mode == InputMode::HistorySearch {
            self.handle_history_search_event(prompt, event)
        } else {
            self.handle_editor_event(prompt, event)
        }
    }

    fn handle_history_search_event(
        &mut self,
        prompt: &dyn Prompt,
        event: ReedlineEvent,
    ) -> io::Result<Option<Signal>> {
        match event {
            ReedlineEvent::CtrlD => {
                if self.editor.is_empty() {
                    self.input_mode = InputMode::Regular;
                    self.editor.reset_undo_stack();
                    Ok(Some(Signal::CtrlD))
                } else {
                    self.run_history_commands(&[EditCommand::Delete]);
                    Ok(None)
                }
            }
            ReedlineEvent::CtrlC => {
                self.input_mode = InputMode::Regular;
                Ok(Some(Signal::CtrlC))
            }
            ReedlineEvent::ClearScreen => Ok(Some(Signal::CtrlL)),
            ReedlineEvent::Enter | ReedlineEvent::HandleTab => {
                if let Some(string) = self.history.string_at_cursor() {
                    self.editor.set_buffer(string);
                    self.editor.remember_undo_state(true);
                }

                self.input_mode = InputMode::Regular;
                self.full_repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::Edit(commands) => {
                self.run_history_commands(&commands);
                self.repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::Mouse => Ok(None),
            ReedlineEvent::Resize(width, height) => {
                self.painter.handle_resize(width, height);
                self.full_repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::Repaint => {
                if self.input_mode != InputMode::HistorySearch {
                    self.full_repaint(prompt)?;
                }
                Ok(None)
            }
            ReedlineEvent::PreviousHistory | ReedlineEvent::Up | ReedlineEvent::SearchHistory => {
                self.history.back();
                self.repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::NextHistory | ReedlineEvent::Down => {
                self.history.forward();
                // Hacky way to ensure that we don't fall of into failed search going forward
                if self.history.string_at_cursor().is_none() {
                    self.history.back();
                }
                self.repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::Paste(_) => {
                // No history search if a paste event is handled
                Ok(None)
            }
            ReedlineEvent::Multiple(_) => {
                // VI multiplier operations currently not supported in the history search
                Ok(None)
            }
            ReedlineEvent::None => {
                // Default no operation
                Ok(None)
            }
        }
    }

    fn handle_editor_event(
        &mut self,
        prompt: &dyn Prompt,
        event: ReedlineEvent,
    ) -> io::Result<Option<Signal>> {
        match event {
            ReedlineEvent::HandleTab => {
                let line_buffer = self.editor.line_buffer();

                let current_hint = self.hinter.current_hint();
                if !current_hint.is_empty() && self.input_mode == InputMode::Regular {
                    self.editor.clear_to_end();
                    self.run_edit_commands(&[EditCommand::InsertString(current_hint)], prompt)?;
                } else {
                    self.tab_handler.handle(line_buffer);
                }

                self.full_repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::CtrlD => {
                if self.editor.is_empty() {
                    self.editor.reset_undo_stack();
                    Ok(Some(Signal::CtrlD))
                } else {
                    self.run_edit_commands(&[EditCommand::Delete], prompt)?;
                    Ok(None)
                }
            }
            ReedlineEvent::CtrlC => {
                self.run_edit_commands(&[EditCommand::Clear], prompt)?;
                self.editor.reset_undo_stack();
                Ok(Some(Signal::CtrlC))
            }
            ReedlineEvent::ClearScreen => Ok(Some(Signal::CtrlL)),
            ReedlineEvent::Enter => {
                let buffer = self.editor.get_buffer().to_string();
                if matches!(self.validator.validate(&buffer), ValidationResult::Complete) {
                    self.append_to_history();
                    self.run_edit_commands(&[EditCommand::Clear], prompt)?;
                    self.painter.print_crlf()?;
                    self.editor.reset_undo_stack();

                    Ok(Some(Signal::Success(buffer)))
                } else {
                    #[cfg(windows)]
                    {
                        self.run_edit_commands(&[EditCommand::InsertChar('\r')], prompt)?;
                    }
                    self.run_edit_commands(&[EditCommand::InsertChar('\n')], prompt)?;
                    self.painter.adjust_prompt_position(&self.editor)?;
                    self.full_repaint(prompt)?;

                    Ok(None)
                }
            }
            ReedlineEvent::Edit(commands) => {
                self.run_edit_commands(&commands, prompt)?;
                self.repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::Mouse => Ok(None),
            ReedlineEvent::Resize(width, height) => {
                self.painter.handle_resize(width, height);
                self.full_repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::Repaint => {
                if self.input_mode != InputMode::HistorySearch {
                    self.full_repaint(prompt)?;
                }
                Ok(None)
            }
            ReedlineEvent::PreviousHistory => {
                self.previous_history();

                self.painter.adjust_prompt_position(&self.editor)?;
                self.full_repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::NextHistory => {
                self.next_history();

                self.painter.adjust_prompt_position(&self.editor)?;
                self.full_repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::Up => {
                self.up_command();

                self.painter.adjust_prompt_position(&self.editor)?;
                self.full_repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::Down => {
                self.down_command();

                self.painter.adjust_prompt_position(&self.editor)?;
                self.full_repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::SearchHistory => {
                // Make sure we are able to undo the result of a reverse history search
                self.editor.remember_undo_state(true);

                self.enter_history_search();
                self.repaint(prompt)?;
                Ok(None)
            }
            ReedlineEvent::Paste(events) => {
                let mut latest_signal = None;
                // Making sure that only InsertChars are handled during a paste event
                for event in events {
                    if let ReedlineEvent::Edit(commands) = event {
                        for command in commands {
                            match command {
                                EditCommand::InsertChar(c) => self.editor.insert_char(c),
                                x => {
                                    self.run_edit_commands(&[x], prompt)?;
                                }
                            }
                        }
                    } else {
                        latest_signal = self.handle_editor_event(prompt, event)?;
                    }
                }

                self.painter.adjust_prompt_position(&self.editor)?;
                self.full_repaint(prompt)?;
                Ok(latest_signal)
            }
            ReedlineEvent::Multiple(events) => {
                // Making sure that only InsertChars are handled during a paste event
                let latest_signal = events
                    .into_iter()
                    .try_fold(None, |_, event| self.handle_editor_event(prompt, event))?;

                self.painter.adjust_prompt_position(&self.editor)?;
                self.full_repaint(prompt)?;
                Ok(latest_signal)
            }
            ReedlineEvent::None => Ok(None),
        }
    }

    fn append_to_history(&mut self) {
        self.history.append(self.editor.get_buffer());
    }

    fn previous_history(&mut self) {
        if self.input_mode != InputMode::HistoryTraversal {
            self.input_mode = InputMode::HistoryTraversal;
            self.set_history_navigation_based_on_line_buffer();
        }

        self.history.back();
        self.update_buffer_from_history();
    }

    fn next_history(&mut self) {
        if self.input_mode != InputMode::HistoryTraversal {
            self.input_mode = InputMode::HistoryTraversal;
            self.set_history_navigation_based_on_line_buffer();
        }

        self.history.forward();
        self.update_buffer_from_history();
    }

    /// Enable the search and navigation through the history from the line buffer prompt
    ///
    /// Enables either prefix search with output in the line buffer or simple traversal
    fn set_history_navigation_based_on_line_buffer(&mut self) {
        if self.editor.is_empty() || self.editor.offset() != self.editor.get_buffer().len() {
            // Perform bash-style basic up/down entry walking
            self.history.set_navigation(HistoryNavigationQuery::Normal(
                // Hack: Tight coupling point to be able to restore previously typed input
                self.editor.line_buffer().clone(),
            ));
        } else {
            // Prefix search like found in fish, zsh, etc.
            // Search string is set once from the current buffer
            // Current setup (code in other methods)
            // Continuing with typing will leave the search
            // but next invocation of this method will start the next search
            let buffer = self.editor.get_buffer().to_string();
            self.history
                .set_navigation(HistoryNavigationQuery::PrefixSearch(buffer));
        }
    }

    /// Switch into reverse history search mode
    ///
    /// This mode uses a separate prompt and handles keybindings sligthly differently!
    fn enter_history_search(&mut self) {
        self.input_mode = InputMode::HistorySearch;
        self.history
            .set_navigation(HistoryNavigationQuery::SubstringSearch("".to_string()));
    }

    /// Dispatches the applicable [`EditCommand`] actions for editing the history search string.
    ///
    /// Only modifies internal state, does not perform regular output!
    fn run_history_commands(&mut self, commands: &[EditCommand]) {
        for command in commands {
            match command {
                EditCommand::InsertChar(c) => {
                    let navigation = self.history.get_navigation();
                    if let HistoryNavigationQuery::SubstringSearch(mut substring) = navigation {
                        substring.push(*c);
                        self.history
                            .set_navigation(HistoryNavigationQuery::SubstringSearch(substring));
                    } else {
                        self.history
                            .set_navigation(HistoryNavigationQuery::SubstringSearch(String::from(
                                *c,
                            )));
                    }
                    self.history.back();
                }
                EditCommand::Backspace => {
                    let navigation = self.history.get_navigation();

                    if let HistoryNavigationQuery::SubstringSearch(substring) = navigation {
                        let new_substring = text_manipulation::remove_last_grapheme(&substring);

                        self.history
                            .set_navigation(HistoryNavigationQuery::SubstringSearch(
                                new_substring.to_string(),
                            ));
                        self.history.back();
                    }
                }
                _ => {
                    self.input_mode = InputMode::Regular;
                }
            }
        }
    }

    /// Set the buffer contents for history traversal/search in the standard prompt
    ///
    /// When using the up/down traversal or fish/zsh style prefix search update the main line buffer accordingly.
    /// Not used for the separate modal reverse search!
    fn update_buffer_from_history(&mut self) {
        match self.history.get_navigation() {
            HistoryNavigationQuery::Normal(original) => {
                if let Some(buffer_to_paint) = self.history.string_at_cursor() {
                    self.editor.set_buffer(buffer_to_paint.clone());
                    self.set_offset(buffer_to_paint.len());
                } else {
                    // Hack
                    self.editor.set_line_buffer(original);
                }
            }
            HistoryNavigationQuery::PrefixSearch(prefix) => {
                if let Some(prefix_result) = self.history.string_at_cursor() {
                    self.editor.set_buffer(prefix_result.clone());
                    self.set_offset(prefix_result.len());
                } else {
                    self.editor.set_buffer(prefix.clone());
                    self.set_offset(prefix.len());
                }
            }
            HistoryNavigationQuery::SubstringSearch(_) => todo!(),
        }
    }

    /// Executes [`EditCommand`] actions by modifying the internal state appropriately. Does not output itself.
    fn run_edit_commands(
        &mut self,
        commands: &[EditCommand],
        prompt: &dyn Prompt,
    ) -> io::Result<()> {
        if self.input_mode == InputMode::HistoryTraversal {
            if matches!(
                self.history.get_navigation(),
                HistoryNavigationQuery::Normal(_)
            ) {
                if let Some(string) = self.history.string_at_cursor() {
                    self.editor.set_buffer(string);
                }
            }
            self.input_mode = InputMode::Regular;
        }

        // Run the commands over the edit buffer
        for command in commands {
            match command {
                EditCommand::MoveToStart => self.editor.move_to_start(),
                EditCommand::MoveToEnd => self.editor.move_to_end(),
                EditCommand::MoveToLineStart => self.editor.move_to_line_start(),
                EditCommand::MoveToLineEnd => self.editor.move_to_line_end(),
                EditCommand::MoveLeft => self.editor.move_left(),
                EditCommand::MoveRight => self.editor.move_right(),
                EditCommand::MoveWordLeft => self.editor.move_word_left(),
                EditCommand::MoveWordRight => self.editor.move_word_right(),
                // Performing mutation here might incur a perf hit down this line when
                // we would like to do multiple inserts.
                // A simple solution that we can do is to queue up these and perform the wrapping
                // check after the loop finishes. Will need to sort out the details.
                EditCommand::InsertChar(c) => {
                    self.editor.insert_char(*c);

                    if self.painter.require_wrapping(&self.editor) {
                        self.handle_wrap(prompt)?;
                    }

                    self.repaint(prompt)?;
                }
                EditCommand::InsertString(s) => {
                    for c in s.chars() {
                        self.editor.insert_char(c);
                    }

                    if self.painter.require_wrapping(&self.editor) {
                        self.handle_wrap(prompt)?;
                    }

                    self.repaint(prompt)?;
                }
                EditCommand::Backspace => self.editor.backspace(),
                EditCommand::Delete => self.editor.delete(),
                EditCommand::BackspaceWord => self.editor.backspace_word(),
                EditCommand::DeleteWord => self.editor.delete_word(),
                EditCommand::Clear => self.editor.clear(),
                EditCommand::ClearToLineEnd => self.editor.clear_to_line_end(),
                EditCommand::CutCurrentLine => self.editor.cut_current_line(),
                EditCommand::CutFromStart => self.editor.cut_from_start(),
                EditCommand::CutToEnd => self.editor.cut_from_end(),
                EditCommand::CutWordLeft => self.editor.cut_word_left(),
                EditCommand::CutWordRight => self.editor.cut_word_right(),
                EditCommand::PasteCutBufferBefore => self.editor.insert_cut_buffer_before(),
                EditCommand::PasteCutBufferAfter => self.editor.insert_cut_buffer_after(),
                EditCommand::UppercaseWord => self.editor.uppercase_word(),
                EditCommand::LowercaseWord => self.editor.lowercase_word(),
                EditCommand::CapitalizeChar => self.editor.capitalize_char(),
                EditCommand::SwapWords => self.editor.swap_words(),
                EditCommand::SwapGraphemes => self.editor.swap_graphemes(),
                EditCommand::Undo => self.editor.undo(),
                EditCommand::Redo => self.editor.redo(),
                EditCommand::CutRightUntil(c) => self.editor.cut_right_until_char(*c, false),
                EditCommand::CutRightBefore(c) => self.editor.cut_right_until_char(*c, true),
                EditCommand::MoveRightUntil(c) => self.editor.move_right_until_char(*c, false),
                EditCommand::MoveRightBefore(c) => self.editor.move_right_until_char(*c, true),
                EditCommand::CutLeftUntil(c) => self.editor.cut_left_until_char(*c, false),
                EditCommand::CutLeftBefore(c) => self.editor.cut_left_until_char(*c, true),
                EditCommand::MoveLeftUntil(c) => self.editor.move_left_until_char(*c, false),
                EditCommand::MoveLeftBefore(c) => self.editor.move_left_until_char(*c, true),
                EditCommand::CutFromLineStart => self.editor.cut_from_line_start(),
                EditCommand::CutToLineEnd => self.editor.cut_to_line_end(),
            }

            match command.undo_behavior() {
                UndoBehavior::Ignore => {}
                UndoBehavior::Full => {
                    self.editor.remember_undo_state(true);
                }
                UndoBehavior::Coalesce => {
                    self.editor.remember_undo_state(false);
                }
            }
        }

        Ok(())
    }

    /// Set the cursor position as understood by the underlying [`LineBuffer`] for the current line
    fn set_offset(&mut self, pos: usize) {
        self.editor.set_insertion_point(pos);
    }

    fn up_command(&mut self) {
        // If we're at the top, then:
        if self.editor.is_cursor_at_first_line() {
            // If we're at the top, move to previous history
            self.previous_history();
        } else {
            self.editor.move_line_up();
        }
    }

    fn down_command(&mut self) {
        // If we're at the top, then:
        if self.editor.is_cursor_at_last_line() {
            // If we're at the top, move to previous history
            self.next_history();
        } else {
            self.editor.move_line_down();
        }
    }

    /// *Partial* repaint of either the buffer or the parts for reverse history search
    fn repaint(&mut self, prompt: &dyn Prompt) -> io::Result<()> {
        // Repainting
        if self.input_mode == InputMode::HistorySearch {
            self.history_search_paint(prompt)?;
        } else {
            self.buffer_paint(prompt)?;
        }

        Ok(())
    }

    /// Repaint logic for the history reverse search
    ///
    /// Overwrites the prompt indicator and highlights the search string
    /// separately from the result bufer.
    fn history_search_paint(&mut self, prompt: &dyn Prompt) -> Result<()> {
        let navigation = self.history.get_navigation();

        if let HistoryNavigationQuery::SubstringSearch(substring) = navigation {
            let status = if !substring.is_empty() && self.history.string_at_cursor().is_none() {
                PromptHistorySearchStatus::Failing
            } else {
                PromptHistorySearchStatus::Passing
            };

            let prompt_history_search = PromptHistorySearch::new(status, substring);

            self.painter.queue_history_search_indicator(
                prompt,
                prompt_history_search,
                self.use_ansi_coloring,
            )?;

            match self.history.string_at_cursor() {
                Some(string) => {
                    self.painter
                        .queue_history_search_result(&string, string.len())?;
                    self.painter.flush()?;
                }

                None => {
                    self.painter.clear_until_newline()?;
                }
            }
        }

        Ok(())
    }

    /// Based on the current buffer create the ansi styled content that shall be painted
    ///
    /// # Returns:
    /// (highlighted_line, hint)
    fn prepare_buffer_content(&mut self, prompt: &dyn Prompt) -> ((String, String), String) {
        let cursor_position_in_buffer = self.editor.offset();
        let buffer_to_paint = self.editor.get_buffer();

        let highlighted_line = self
            .highlighter
            .highlight(buffer_to_paint)
            .render_around_insertion_point(
                cursor_position_in_buffer,
                prompt.render_prompt_multiline_indicator().borrow(),
                self.use_ansi_coloring,
            );

        let hint: String = if self.input_mode == InputMode::Regular {
            self.hinter.handle(
                buffer_to_paint,
                cursor_position_in_buffer,
                self.history.as_ref(),
                self.use_ansi_coloring,
            )
        } else {
            String::new()
        };

        (highlighted_line, hint)
    }

    /// Repaint logic for the normal input prompt buffer
    ///
    /// Requires coordinates where the input buffer begins after the prompt.
    /// Performs highlighting and hinting at the moment!
    fn buffer_paint(&mut self, prompt: &dyn Prompt) -> Result<()> {
        let (highlighted_line, hint) = self.prepare_buffer_content(prompt);

        self.painter.queue_buffer(highlighted_line, hint)?;
        self.painter.flush()?;

        Ok(())
    }

    /// Triggers a full repaint including the prompt parts
    ///
    /// Includes the highlighting and hinting calls.
    fn full_repaint(&mut self, prompt: &dyn Prompt) -> Result<()> {
        let prompt_mode = self.prompt_edit_mode();
        let (highlighted_line, hint) = self.prepare_buffer_content(prompt);

        self.painter.repaint_everything(
            prompt,
            prompt_mode,
            highlighted_line,
            hint,
            self.use_ansi_coloring,
        )?;

        Ok(())
    }

    fn handle_wrap(&mut self, prompt: &dyn Prompt) -> io::Result<()> {
        let (highlighted_line, hint) = self.prepare_buffer_content(prompt);

        self.painter.wrap(highlighted_line, hint)
    }
}
