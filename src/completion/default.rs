use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    str::Chars,
};

use crate::{Completer, Span};

/// A history-specific completer with a focus on completing whole lines of history
pub struct HistoryCompleter {
    history: Vec<String>,
}

impl HistoryCompleter {
    pub fn new(history: Vec<String>) -> Self {
        Self { history }
    }
}

impl Completer for HistoryCompleter {
    fn complete(&self, line: &str, pos: usize) -> Vec<(Span, String)> {
        let mut completions = vec![];

        if line.is_empty() {
            return vec![];
        }

        for hist in &self.history {
            if hist.starts_with(&line[0..pos]) {
                completions.push((
                    Span {
                        start: pos,
                        end: line.len(),
                    },
                    hist[pos..].to_string(),
                ));
            }
        }

        if let Some(last) = completions.last() {
            vec![last.clone()]
        } else {
            completions
        }
    }
}

/// A default completer that can detect keywords
///
/// # Example
///
/// ```rust, no_run
/// use reedline::{DefaultCompleter, CircularCompletionHandler, Reedline};
///
/// let commands = vec![
///  "test".into(),
///  "hello world".into(),
///  "hello world reedline".into(),
///  "this is the reedline crate".into(),
/// ];
/// let completer = Box::new(DefaultCompleter::new_with_wordlen(commands.clone(), 2));
///
/// let mut line_editor = Reedline::create().unwrap()
///         .with_completion_action_handler(Box::new(
///   CircularCompletionHandler::default().with_completer(completer),
/// ));
/// ```
#[derive(Debug, Clone)]
pub struct DefaultCompleter {
    root: CompletionNode,
    min_word_len: usize,
}

impl Default for DefaultCompleter {
    fn default() -> Self {
        let inclusions = Rc::new(BTreeSet::new());
        Self {
            root: CompletionNode::new(inclusions),
            min_word_len: 2,
        }
    }
}
impl Completer for DefaultCompleter {
    /// Returns a vector of completions and the position in which they must be replaced;
    /// based on the provided input.
    ///
    /// # Arguments
    ///
    /// * `line`    The line to complete
    /// * `pos`   The cursor position
    ///
    /// # Example
    /// ```
    /// use reedline::{DefaultCompleter,Completer,Span};
    ///
    /// let mut completions = DefaultCompleter::default();
    /// completions.insert(vec!["batman","robin","batmobile","batcave","robber"].iter().map(|s| s.to_string()).collect());
    /// assert_eq!(
    ///     completions.complete("bat",3),
    ///     vec![
    ///         (Span { start: 0, end: 3 }, "batcave".into()),
    ///         (Span { start: 0, end: 3 }, "batman".into()),
    ///         (Span { start: 0, end: 3 }, "batmobile".into()),
    ///     ]);
    ///
    /// assert_eq!(
    ///     completions.complete("to the bat",10),
    ///     vec![
    ///         (Span { start: 7, end: 10 }, "batcave".into()),
    ///         (Span { start: 7, end: 10 }, "batman".into()),
    ///         (Span { start: 7, end: 10 }, "batmobile".into()),
    ///     ]);
    /// ```
    fn complete(&self, line: &str, pos: usize) -> Vec<(Span, String)> {
        let mut span_line_whitespaces = 0;
        let mut completions = vec![];
        if !line.is_empty() {
            let mut splitted = line[0..pos].split(' ').rev();
            let mut span_line: String = String::new();
            for _ in 0..splitted.clone().count() {
                if let Some(s) = splitted.next() {
                    if s.is_empty() {
                        span_line_whitespaces += 1;
                        continue;
                    }
                    if span_line.is_empty() {
                        span_line = s.to_string();
                    } else {
                        span_line = format!("{} {}", s, span_line);
                    }
                    if let Some(mut extensions) = self.root.complete(span_line.chars()) {
                        extensions.sort();
                        completions.extend(
                            extensions
                                .iter()
                                .map(|ext| {
                                    (
                                        Span::new(
                                            pos - span_line.len() - span_line_whitespaces,
                                            pos,
                                        ),
                                        format!("{}{}", span_line, ext),
                                    )
                                })
                                .filter(|t| t.1.len() > (t.0.end - t.0.start))
                                .collect::<Vec<(Span, String)>>(),
                        );
                    }
                }
            }
        }
        completions.dedup();
        completions
    }
}
impl DefaultCompleter {
    /// Construct the default completer with a list of commands/keywords to highlight
    pub fn new(external_commands: Vec<String>) -> Self {
        let mut dc = DefaultCompleter::default();
        dc.insert(external_commands);
        dc
    }

    /// Construct the default completer with a list of commands/keywords to highlight, given a minimum word length
    pub fn new_with_wordlen(external_commands: Vec<String>, min_word_len: usize) -> Self {
        let mut dc = DefaultCompleter::default().set_min_word_len(min_word_len);
        dc.insert(external_commands);
        dc
    }

    /// Insert `external_commands` list in the object root
    ///
    /// # Arguments
    ///
    /// * `line`    A vector of `String` containing the external commands
    ///
    /// # Example
    /// ```
    /// use reedline::{DefaultCompleter,Completer};
    ///
    /// let mut completions = DefaultCompleter::default();
    ///
    /// // Insert multiple words
    /// completions.insert(vec!["a","line","with","many","words"].iter().map(|s| s.to_string()).collect());
    ///
    /// // The above line is equal to the following:
    /// completions.insert(vec!["a","line","with"].iter().map(|s| s.to_string()).collect());
    /// completions.insert(vec!["many","words"].iter().map(|s| s.to_string()).collect());
    /// ```
    pub fn insert(&mut self, words: Vec<String>) {
        for word in words {
            if word.len() >= self.min_word_len {
                self.root.insert(word.chars());
            }
        }
    }

    /// Create a new `DefaultCompleter` with provided non alphabet characters whitelisted.
    /// The default `DefaultCompleter` will only parse alphabet characters (a-z, A-Z). Use this to
    /// introduce additional accepted special characters.
    ///
    /// # Arguments
    ///
    /// * `incl`    An array slice with allowed characters
    ///
    /// # Example
    /// ```
    /// use reedline::{DefaultCompleter,Completer,Span};
    ///
    /// let mut completions = DefaultCompleter::default();
    /// completions.insert(vec!["test-hyphen","test_underscore"].iter().map(|s| s.to_string()).collect());
    /// assert_eq!(
    ///     completions.complete("te",2),
    ///     vec![(Span { start: 0, end: 2 }, "test".into())]);
    ///
    /// let mut completions = DefaultCompleter::with_inclusions(&['-', '_']);
    /// completions.insert(vec!["test-hyphen","test_underscore"].iter().map(|s| s.to_string()).collect());
    /// assert_eq!(
    ///     completions.complete("te",2),
    ///     vec![
    ///         (Span { start: 0, end: 2 }, "test-hyphen".into()),
    ///         (Span { start: 0, end: 2 }, "test_underscore".into()),
    ///     ]);
    /// ```
    pub fn with_inclusions(incl: &[char]) -> Self {
        let mut set = BTreeSet::new();
        set.extend(incl.iter());
        let inclusions = Rc::new(set);
        Self {
            root: CompletionNode::new(inclusions),
            ..Self::default()
        }
    }

    /// Clears all the data from the tree
    /// # Example
    /// ```
    /// use reedline::{DefaultCompleter,Completer};
    ///
    /// let mut completions = DefaultCompleter::default();
    /// completions.insert(vec!["batman","robin","batmobile","batcave","robber"].iter().map(|s| s.to_string()).collect());
    /// assert_eq!(completions.word_count(), 5);
    /// assert_eq!(completions.size(), 24);
    /// completions.clear();
    /// assert_eq!(completions.size(), 1);
    /// assert_eq!(completions.word_count(), 0);
    /// ```
    pub fn clear(&mut self) {
        self.root.clear();
    }

    /// Returns a count of how many words that exist in the tree
    /// # Example
    /// ```
    /// use reedline::{DefaultCompleter,Completer};
    ///
    /// let mut completions = DefaultCompleter::default();
    /// completions.insert(vec!["batman","robin","batmobile","batcave","robber"].iter().map(|s| s.to_string()).collect());
    /// assert_eq!(completions.word_count(), 5);
    /// ```
    pub fn word_count(&self) -> u32 {
        self.root.word_count()
    }

    /// Returns the size of the tree, the amount of nodes, not words
    /// # Example
    /// ```
    /// use reedline::{DefaultCompleter,Completer};
    ///
    /// let mut completions = DefaultCompleter::default();
    /// completions.insert(vec!["batman","robin","batmobile","batcave","robber"].iter().map(|s| s.to_string()).collect());
    /// assert_eq!(completions.size(), 24);
    /// ```
    pub fn size(&self) -> u32 {
        self.root.subnode_count()
    }

    /// Returns the minimum word length to complete. This allows you
    /// to pass full sentences to `insert()` and not worry about
    /// pruning out small words like "a" or "to", because they will be
    /// ignored.
    /// # Example
    /// ```
    /// use reedline::{DefaultCompleter,Completer};
    ///
    /// let mut completions = DefaultCompleter::default().set_min_word_len(4);
    /// completions.insert(vec!["one","two","three","four","five"].iter().map(|s| s.to_string()).collect());
    /// assert_eq!(completions.word_count(), 3);
    ///
    /// let mut completions = DefaultCompleter::default().set_min_word_len(1);
    /// completions.insert(vec!["one","two","three","four","five"].iter().map(|s| s.to_string()).collect());
    /// assert_eq!(completions.word_count(), 5);
    /// ```
    pub fn min_word_len(&self) -> usize {
        self.min_word_len
    }

    /// Sets the minimum word length to complete on. Smaller words are
    /// ignored. This only affects future calls to `insert()` -
    /// changing this won't start completing on smaller words that
    /// were added in the past, nor will it exclude larger words
    /// already inserted into the completion tree.
    pub fn set_min_word_len(mut self, len: usize) -> DefaultCompleter {
        self.min_word_len = len;
        self
    }
}

#[derive(Debug, Clone)]
struct CompletionNode {
    subnodes: BTreeMap<char, CompletionNode>,
    leaf: bool,
    inclusions: Rc<BTreeSet<char>>,
}

impl CompletionNode {
    fn new(incl: Rc<BTreeSet<char>>) -> Self {
        Self {
            subnodes: BTreeMap::new(),
            leaf: false,
            inclusions: incl,
        }
    }

    fn clear(&mut self) {
        self.subnodes.clear();
    }

    fn word_count(&self) -> u32 {
        let mut count = self.subnodes.values().map(CompletionNode::word_count).sum();
        if self.leaf {
            count += 1;
        }
        count
    }

    fn subnode_count(&self) -> u32 {
        self.subnodes
            .values()
            .map(CompletionNode::subnode_count)
            .sum::<u32>()
            + 1
    }

    fn insert(&mut self, mut iter: Chars) {
        if let Some(c) = iter.next() {
            if self.inclusions.contains(&c) || c.is_alphanumeric() || c.is_whitespace() {
                let inclusions = self.inclusions.clone();
                let subnode = self
                    .subnodes
                    .entry(c)
                    .or_insert_with(|| CompletionNode::new(inclusions));
                subnode.insert(iter);
            } else {
                self.leaf = true;
            }
        } else {
            self.leaf = true;
        }
    }

    fn complete(&self, mut iter: Chars) -> Option<Vec<String>> {
        if let Some(c) = iter.next() {
            if let Some(subnode) = self.subnodes.get(&c) {
                subnode.complete(iter)
            } else {
                None
            }
        } else {
            Some(self.collect(""))
        }
    }

    fn collect(&self, partial: &str) -> Vec<String> {
        let mut completions = vec![];
        if self.leaf {
            completions.push(partial.to_string());
        }

        if !self.subnodes.is_empty() {
            for (c, node) in &self.subnodes {
                let mut partial = partial.to_string();
                partial.push(*c);
                completions.append(&mut node.collect(&partial));
            }
        }
        completions
    }
}
