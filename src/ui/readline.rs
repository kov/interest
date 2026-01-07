//! Readline wrapper with simple command completion.

use std::path::PathBuf;

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Config, Context, Editor, Helper};

pub struct CommandHelper {
    patterns: Vec<Vec<String>>,
    hinter: HistoryHinter,
}

impl CommandHelper {
    pub fn new(patterns: &[&[&str]]) -> Self {
        Self {
            patterns: patterns
                .iter()
                .map(|p| p.iter().map(|s| s.to_string()).collect())
                .collect(),
            hinter: HistoryHinter::default(),
        }
    }
}

impl Helper for CommandHelper {}
impl Validator for CommandHelper {}
impl Highlighter for CommandHelper {}

impl Hinter for CommandHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<String> {
        self.hinter.hint(line, pos, ctx)
    }
}

impl Completer for CommandHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let before = &line[..pos];
        let mut tokens: Vec<&str> = before.split_whitespace().collect();

        // Treat trailing space as start of a new token
        if before.chars().last().is_some_and(|c| c.is_whitespace()) {
            tokens.push("");
        }

        let prefix = tokens.last().copied().unwrap_or("");
        let start = pos.saturating_sub(prefix.len());

        let has_leading_slash = tokens.first().map(|t| t.starts_with('/')).unwrap_or(false);

        let fixed_tokens: Vec<String> = tokens
            .iter()
            .take(tokens.len().saturating_sub(1))
            .map(|t| t.trim_start_matches('/').to_lowercase())
            .collect();
        let prefix_lower = prefix.trim_start_matches('/').to_lowercase();

        let mut matches = Vec::new();

        for pattern in &self.patterns {
            if pattern.len() < tokens.len() {
                continue;
            }

            // Ensure already-typed tokens match pattern prefix
            if fixed_tokens.iter().enumerate().any(|(idx, typed)| {
                pattern.get(idx).map(|p| p.eq_ignore_ascii_case(typed)) != Some(true)
            }) {
                continue;
            }

            let candidate = &pattern[tokens.len() - 1];
            if !candidate.to_lowercase().starts_with(&prefix_lower) {
                continue;
            }

            // Replace only the current token to avoid duplicating earlier tokens
            let completing_first_token = tokens.len() == 1;
            let candidate_token = if completing_first_token && has_leading_slash {
                format!("/{}", candidate)
            } else {
                candidate.to_string()
            };

            let replacement = format!("{} ", candidate_token);

            matches.push(Pair {
                display: replacement.clone(),
                replacement,
            });
        }

        // Deduplicate identical replacements (rustyline may propose duplicates when
        // multiple patterns map to the same token)
        matches.sort_by(|a, b| a.replacement.cmp(&b.replacement));
        matches.dedup_by(|a, b| a.replacement == b.replacement);

        Ok((start, matches))
    }
}

/// Thin wrapper over `rustyline::Editor` with preset commands and history path.
pub struct Readline {
    editor: Editor<CommandHelper, DefaultHistory>,
    history_path: PathBuf,
}

impl Readline {
    pub fn new(
        command_patterns: &[&[&str]],
        history_path: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        let config = Config::builder()
            .history_ignore_dups(true)?
            .history_ignore_space(true)
            .build();
        let helper = CommandHelper::new(command_patterns);
        let mut editor = Editor::with_config(config)?;
        editor.set_helper(Some(helper));

        let history_path = history_path.unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".interest/.history")
        });

        let _ = editor.load_history(&history_path);

        Ok(Self {
            editor,
            history_path,
        })
    }

    pub fn readline(&mut self, prompt: &str) -> Result<String, ReadlineError> {
        let line = self.editor.readline(prompt)?;
        if !line.trim().is_empty() {
            let _ = self.editor.add_history_entry(line.as_str());
            let _ = self.editor.append_history(&self.history_path);
        }
        Ok(line)
    }

    /// Utility for tests to inspect completions without invoking terminal input.
    pub fn completions(&self, line: &str) -> Vec<String> {
        if let Some(helper) = self.editor.helper() {
            let pos = line.len();
            let history = self.editor.history();
            if let Ok((_, pairs)) = helper.complete(line, pos, &Context::new(history)) {
                return pairs.into_iter().map(|p| p.replacement).collect();
            }
        }
        Vec::new()
    }

    /// Return completions alongside the replacement start index (for tests).
    pub fn completions_with_start(&self, line: &str) -> Vec<(usize, String)> {
        if let Some(helper) = self.editor.helper() {
            let pos = line.len();
            let history = self.editor.history();
            if let Ok((start, pairs)) = helper.complete(line, pos, &Context::new(history)) {
                return pairs.into_iter().map(|p| (start, p.replacement)).collect();
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_completer_suggests_import() {
        let tmp = std::env::temp_dir().join("interest_history_test");
        let _ = fs::remove_file(&tmp);
        let rl = Readline::new(
            &[&["import"], &["portfolio", "show"], &["tax", "summary"]],
            Some(tmp),
        )
        .unwrap();
        let completions = rl.completions("/i");
        assert!(completions.contains(&"/import ".to_string()));
    }

    #[test]
    fn test_completer_handles_multi_token() {
        let tmp = std::env::temp_dir().join("interest_history_test_multi");
        let _ = fs::remove_file(&tmp);
        let rl = Readline::new(&[&["tax", "summary"], &["tax", "report"]], Some(tmp)).unwrap();
        let completions = rl.completions_with_start("ta");
        assert_eq!(completions, vec![(0, "tax ".to_string())]);

        let completions = rl.completions_with_start("tax su");
        assert_eq!(completions, vec![(4, "summary ".to_string())]);

        let completions = rl.completions_with_start("/tax su");
        assert_eq!(completions, vec![(5, "summary ".to_string())]);

        let bad = rl.completions_with_start("tax foo");
        assert!(bad.is_empty());
    }
}
