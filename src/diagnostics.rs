use crate::lexer::token::Span;
use std::fmt;

#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
    pub span: Span,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "error at line {}, column {}: {}",
            self.span.line, self.span.column, self.message
        )
    }
}

impl std::error::Error for CompileError {}

pub fn report_error(source: &str, error: &CompileError) {
    let lines: Vec<&str> = source.lines().collect();
    let line_idx = error.span.line.saturating_sub(1);

    eprintln!("\x1b[1;31merror\x1b[0m: {}", error.message);

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
            error.span.end - error.span.start
        } else {
            1
        };

        eprintln!(
            " {:>padding$} \x1b[1;34m|\x1b[0m {}\x1b[1;31m{}\x1b[0m",
            "",
            " ".repeat(col),
            "^".repeat(underline_len),
            padding = padding
        );
    }
}
