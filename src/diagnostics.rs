use crate::lexer::token::Span;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Severity {
    Error,
    Warning,
    Hint,
}

#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
    pub span: Span,
    pub severity: Severity,
    pub hint: Option<String>,
}

impl CompileError {
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            severity: Severity::Error,
            hint: None,
        }
    }

    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            severity: Severity::Warning,
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Hint => "hint",
        };
        write!(
            f,
            "{} at line {}, column {}: {}",
            label, self.span.line, self.span.column, self.message
        )
    }
}

impl std::error::Error for CompileError {}

/// Accumulates multiple diagnostics instead of stopping at the first error.
pub struct DiagnosticBag {
    pub errors: Vec<CompileError>,
}

impl DiagnosticBag {
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    pub fn add(&mut self, error: CompileError) {
        self.errors.push(error);
    }

    pub fn has_errors(&self) -> bool {
        self.errors.iter().any(|e| e.severity == Severity::Error)
    }

    pub fn error_count(&self) -> usize {
        self.errors
            .iter()
            .filter(|e| e.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.errors
            .iter()
            .filter(|e| e.severity == Severity::Warning)
            .count()
    }
}

pub fn report_error(source: &str, filename: &str, error: &CompileError) {
    let lines: Vec<&str> = source.lines().collect();
    let line_idx = error.span.line.saturating_sub(1);

    let (color, label) = match error.severity {
        Severity::Error => ("\x1b[1;31m", "error"),
        Severity::Warning => ("\x1b[1;33m", "warning"),
        Severity::Hint => ("\x1b[1;36m", "hint"),
    };

    eprintln!("{}{}\x1b[0m: {}", color, label, error.message);
    eprintln!(
        "  \x1b[1;34m-->\x1b[0m {}:{}:{}",
        filename, error.span.line, error.span.column
    );

    if line_idx < lines.len() {
        let line_content = lines[line_idx];
        let line_num = error.span.line;
        let padding = line_num.to_string().len();

        eprintln!(" {:>padding$} \x1b[1;34m|\x1b[0m", "", padding = padding);
        eprintln!(
            " \x1b[1;34m{}\x1b[0m \x1b[1;34m|\x1b[0m {}",
            line_num, line_content
        );

        let col = error.span.column.saturating_sub(1);
        let underline_len = if error.span.end > error.span.start {
            (error.span.end - error.span.start).min(line_content.len().saturating_sub(col))
        } else {
            1
        };

        eprintln!(
            " {:>padding$} \x1b[1;34m|\x1b[0m {}{}{}{}",
            "",
            " ".repeat(col),
            color,
            "^".repeat(underline_len.max(1)),
            "\x1b[0m",
            padding = padding
        );

        if let Some(hint) = &error.hint {
            eprintln!(" {:>padding$} \x1b[1;34m|\x1b[0m", "", padding = padding);
            eprintln!(
                " {:>padding$} \x1b[1;34m= \x1b[1;36mhint\x1b[0m: {}",
                "",
                hint,
                padding = padding
            );
        }
    }

    eprintln!();
}

pub fn report_all(source: &str, filename: &str, bag: &DiagnosticBag) {
    for error in &bag.errors {
        report_error(source, filename, error);
    }

    let errors = bag.error_count();
    let warnings = bag.warning_count();

    if errors > 0 || warnings > 0 {
        let mut parts = Vec::new();
        if errors > 0 {
            parts.push(format!(
                "\x1b[1;31m{} error{}\x1b[0m",
                errors,
                if errors == 1 { "" } else { "s" }
            ));
        }
        if warnings > 0 {
            parts.push(format!(
                "\x1b[1;33m{} warning{}\x1b[0m",
                warnings,
                if warnings == 1 { "" } else { "s" }
            ));
        }
        eprintln!("{} emitted", parts.join(", "));
        eprintln!();
    }
}
