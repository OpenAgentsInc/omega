//! `read` tool: schema validation, range selection, and line formatting.

use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::ToolError;
use crate::roots::ReadRoots;

/// Default and maximum line count for a single `read` call.
pub const DEFAULT_LIMIT: u64 = 2000;
pub const MAX_LIMIT: u64 = 2000;

/// Maximum encoded `content` size (bytes) before early stop with `truncated: true`.
/// Documented in the crate README.
pub const MAX_CONTENT_BYTES: usize = 1_048_576;

const DEFAULT_LINE_NUMBER_WIDTH: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadInput {
    pub path: String,
    #[serde(default = "default_offset")]
    pub offset: u64,
    #[serde(default = "default_limit")]
    pub limit: u64,
}

fn default_offset() -> u64 {
    1
}

fn default_limit() -> u64 {
    DEFAULT_LIMIT
}

impl ReadInput {
    pub fn validate(&self) -> Result<(), ToolError> {
        if self.offset < 1 {
            return Err(ToolError::invalid_params("offset must be >= 1", &self.path));
        }
        if self.limit < 1 || self.limit > MAX_LIMIT {
            return Err(ToolError::invalid_params(
                format!("limit must be between 1 and {MAX_LIMIT}"),
                &self.path,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadOutput {
    pub path: String,
    pub content: String,
    pub line_start: Option<u64>,
    pub line_end: Option<u64>,
    pub truncated: bool,
}

/// Parse and validate a `read` input object. Unknown fields and schema violations
/// become `invalid_params`.
pub fn parse_read_input(value: &serde_json::Value) -> Result<ReadInput, ToolError> {
    let path_hint = value
        .get("path")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_owned();
    let input: ReadInput = serde_json::from_value(value.clone()).map_err(|error| {
        ToolError::invalid_params(format!("invalid read params: {error}"), path_hint)
    })?;
    input.validate()?;
    Ok(input)
}

/// Execute `read` against `roots` using the validated input.
pub fn execute_read(roots: &ReadRoots, input: &ReadInput) -> Result<ReadOutput, ToolError> {
    input.validate()?;
    let resolved = roots.resolve_readable_file(&input.path)?;
    let bytes = fs::read(&resolved).map_err(|_| ToolError::read_failed(&input.path))?;
    let text = String::from_utf8(bytes).map_err(|_| ToolError::invalid_text(&input.path))?;
    format_read_output(
        &input.path,
        &text,
        input.offset,
        input.limit,
        MAX_CONTENT_BYTES,
    )
}

/// Format lines for a successful read. `max_content_bytes` is injectable for tests.
pub fn format_read_output(
    path: &str,
    text: &str,
    offset: u64,
    limit: u64,
    max_content_bytes: usize,
) -> Result<ReadOutput, ToolError> {
    if offset < 1 {
        return Err(ToolError::invalid_params("offset must be >= 1", path));
    }
    if limit < 1 || limit > MAX_LIMIT {
        return Err(ToolError::invalid_params(
            format!("limit must be between 1 and {MAX_LIMIT}"),
            path,
        ));
    }

    let lines: Vec<&str> = text.split('\n').collect();
    // `split` yields a trailing empty piece when the file ends with `\n`. Drop it so a
    // final newline does not invent an empty extra line (Codex-style line enumeration).
    let line_count = if text.is_empty() {
        0
    } else if text.ends_with('\n') {
        lines.len().saturating_sub(1)
    } else {
        lines.len()
    };

    let start_index = (offset as usize).saturating_sub(1);
    if start_index >= line_count {
        return Ok(ReadOutput {
            path: path.to_owned(),
            content: String::new(),
            line_start: None,
            line_end: None,
            truncated: false,
        });
    }

    let max_take = limit as usize;
    let available = line_count - start_index;
    let intended_take = max_take.min(available);
    let natural_end_line = offset + (intended_take as u64) - 1;
    let width = line_number_width(natural_end_line);

    let mut display_lines: Vec<String> = Vec::new();
    let mut content_bytes = 0usize;
    let mut truncated = false;
    let mut first_line: Option<u64> = None;
    let mut last_line: Option<u64> = None;

    for index in 0..intended_take {
        let line_number = offset + index as u64;
        let source = lines[start_index + index];
        let display = format_display_line(line_number, width, source);
        let addition = if display_lines.is_empty() {
            display.len()
        } else {
            display.len() + 1
        };
        if content_bytes + addition > max_content_bytes {
            if display_lines.is_empty() {
                return Err(ToolError::response_too_large(path));
            }
            truncated = true;
            break;
        }
        content_bytes += addition;
        if first_line.is_none() {
            first_line = Some(line_number);
        }
        last_line = Some(line_number);
        display_lines.push(display);
    }

    // If we stopped early due to the size bound but more of the requested range remains,
    // truncated is already true. If size bound was not hit, truncated stays false even
    // when the range was clipped at EOF.
    if !truncated && display_lines.len() < intended_take {
        // Unreachable with the loop above; kept for clarity.
    }
    // When size truncation stops before the intended end, and there were more lines in
    // the requested range, ensure truncated is set.
    if !truncated {
        let returned = display_lines.len();
        if returned < intended_take {
            truncated = true;
        }
    }

    Ok(ReadOutput {
        path: path.to_owned(),
        content: display_lines.join("\n"),
        line_start: first_line,
        line_end: last_line,
        truncated,
    })
}

pub fn line_number_width(line_end: u64) -> usize {
    let digits = decimal_digits(line_end);
    digits.max(DEFAULT_LINE_NUMBER_WIDTH)
}

fn decimal_digits(value: u64) -> usize {
    let mut digits = 1usize;
    let mut remaining = value;
    while remaining >= 10 {
        remaining /= 10;
        digits += 1;
    }
    digits
}

pub fn format_display_line(line_number: u64, width: usize, source: &str) -> String {
    format!("{line_number:>width$}\t{source}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ToolErrorCode;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn sample_text() -> &'static str {
        "alpha\nbeta\ngamma\ndelta\n"
    }

    #[test]
    fn formats_requested_line_range() {
        let output = format_read_output("f", sample_text(), 2, 2, MAX_CONTENT_BYTES).expect("ok");
        assert_eq!(output.content, "     2\tbeta\n     3\tgamma");
        assert_eq!(output.line_start, Some(2));
        assert_eq!(output.line_end, Some(3));
        assert!(!output.truncated);
    }

    #[test]
    fn pads_line_numbers_to_six_by_default() {
        let output = format_read_output("f", "alpha\nbeta\n", 1, 2, MAX_CONTENT_BYTES).expect("ok");
        assert_eq!(output.content, "     1\talpha\n     2\tbeta");
        for line in output.content.lines() {
            let (number, _) = line.split_once('\t').expect("tab");
            assert_eq!(number.len(), 6);
        }
    }

    #[test]
    fn pads_line_numbers_beyond_six_digits() {
        // Natural end line 1_000_000 has 7 digits → width 7.
        let mut text = String::new();
        for index in 1..=1_000_000 {
            text.push_str("x\n");
            // Keep memory reasonable: only need lines around the end for selection,
            // but format_read_output enumerates all lines via split. Use a sparse approach:
            // build only enough content for offset near 1_000_000.
            let _ = index;
            if index >= 3 {
                break;
            }
        }
        // Build a file with 1_000_002 lines cheaply: "a\n" * n
        let total = 1_000_002usize;
        let mut big = String::with_capacity(total * 2);
        for _ in 0..total {
            big.push_str("a\n");
        }
        let output = format_read_output("f", &big, 999_999, 3, MAX_CONTENT_BYTES).expect("ok");
        assert_eq!(output.line_start, Some(999_999));
        assert_eq!(output.line_end, Some(1_000_001));
        let first = output.content.lines().next().expect("line");
        let (number, rest) = first.split_once('\t').expect("tab");
        assert_eq!(number.len(), 7);
        assert_eq!(number, " 999999");
        assert_eq!(rest, "a");
    }

    #[test]
    fn offset_past_eof_returns_empty_success() {
        let output = format_read_output("f", "one\ntwo\n", 10, 5, MAX_CONTENT_BYTES).expect("ok");
        assert_eq!(output.content, "");
        assert_eq!(output.line_start, None);
        assert_eq!(output.line_end, None);
        assert!(!output.truncated);
    }

    #[test]
    fn clips_range_at_eof() {
        let output = format_read_output("f", "one\ntwo\n", 2, 50, MAX_CONTENT_BYTES).expect("ok");
        assert_eq!(output.content, "     2\ttwo");
        assert_eq!(output.line_start, Some(2));
        assert_eq!(output.line_end, Some(2));
        assert!(!output.truncated);
    }

    #[test]
    fn preserves_final_line_without_trailing_newline() {
        let output = format_read_output("f", "one\ntwo", 1, 10, MAX_CONTENT_BYTES).expect("ok");
        assert_eq!(output.content, "     1\tone\n     2\ttwo");
        assert_eq!(output.line_end, Some(2));
    }

    #[test]
    fn limit_above_max_is_invalid_params() {
        let input = ReadInput {
            path: "/tmp/x".into(),
            offset: 1,
            limit: 2001,
        };
        let error = input.validate().expect_err("cap");
        assert_eq!(error.code, ToolErrorCode::InvalidParams);
    }

    #[test]
    fn rejects_unknown_fields() {
        let value = serde_json::json!({"path": "/tmp/x", "start_line": 1});
        let error = parse_read_input(&value).expect_err("unknown");
        assert_eq!(error.code, ToolErrorCode::InvalidParams);
    }

    #[test]
    fn response_size_truncation_sets_flag() {
        let text = "aaaa\nbbbb\ncccc\n";
        // Bound that fits first display line only.
        let first = format_display_line(1, 6, "aaaa");
        let max = first.len();
        let output = format_read_output("f", text, 1, 3, max).expect("ok");
        assert!(output.truncated);
        assert_eq!(output.line_start, Some(1));
        assert_eq!(output.line_end, Some(1));
        assert_eq!(output.content, first);
    }

    #[test]
    fn response_too_large_when_first_line_does_not_fit() {
        let text = "toolongline\n";
        let error = format_read_output("f", text, 1, 1, 4).expect_err("too large");
        assert_eq!(error.code, ToolErrorCode::ResponseTooLarge);
    }

    #[test]
    fn invalid_utf8_is_invalid_text() {
        let directory = tempdir().expect("temp");
        let file = directory.path().join("bin.dat");
        fs::write(&file, [0x80, 0x81, 0x82]).expect("write");
        let roots =
            ReadRoots::new([directory.path().to_path_buf()], directory.path()).expect("roots");
        let input = ReadInput {
            path: file.to_string_lossy().into_owned(),
            offset: 1,
            limit: 10,
        };
        let error = execute_read(&roots, &input).expect_err("utf8");
        assert_eq!(error.code, ToolErrorCode::InvalidText);
    }

    #[test]
    fn execute_read_happy_path() {
        let directory = tempdir().expect("temp");
        let file = directory.path().join("sample.txt");
        fs::write(&file, "alpha\nbeta\n").expect("write");
        let roots =
            ReadRoots::new([directory.path().to_path_buf()], directory.path()).expect("roots");
        let input = ReadInput {
            path: file.to_string_lossy().into_owned(),
            offset: 1,
            limit: 2,
        };
        let output = execute_read(&roots, &input).expect("read");
        assert_eq!(output.content, "     1\talpha\n     2\tbeta");
    }

    #[test]
    fn symlink_escape_rejected_through_execute_read() {
        let directory = tempdir().expect("temp");
        let outside = tempdir().expect("outside");
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "nope\n").expect("write");
        let link = directory.path().join("escape");
        symlink(&secret, &link).expect("symlink");
        let roots =
            ReadRoots::new([directory.path().to_path_buf()], directory.path()).expect("roots");
        let input = ReadInput {
            path: link.to_string_lossy().into_owned(),
            offset: 1,
            limit: 10,
        };
        let error = execute_read(&roots, &input).expect_err("escape");
        assert_eq!(error.code, ToolErrorCode::PathNotAllowed);
    }
}
