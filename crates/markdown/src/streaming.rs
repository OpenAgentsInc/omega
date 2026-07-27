//! What to append to markdown that has not finished arriving, so that it
//! renders as the thing it is going to be rather than as its own syntax.
//!
//! OMEGA-DELTA-0128. While a model streams, the source is a prefix.
//! `**Searching` is on its way to `**Searching**`, but the closing `**` has not
//! arrived yet and a correct markdown parser has no choice: it renders the
//! asterisks literally. So the reader watches raw syntax appear and then snap
//! into styled text, once per construct, for the length of the message.
//!
//! Streamdown (`packages/remend`) answers this by repairing the source *before*
//! parsing — it appends the missing `**`, closes the backtick, gives an
//! unfinished link a placeholder destination — and switches the repair off for
//! the final, non-streaming render so that nothing invented outlives the
//! stream. We take that answer, with two differences that this renderer forces:
//!
//! 1. **Every repair is a suffix.** The rendered output carries byte ranges
//!    back into the source, for selection, copy-as-markdown, click-to-source,
//!    autoscroll and search highlights. Streamdown rewrites in the middle of
//!    the string: it deletes an unfinished image, strips a half-typed HTML tag,
//!    moves a `_` in front of a trailing newline. Every one of those shifts the
//!    offset of everything after it, and here that silently misaddresses all of
//!    the above. So the repair may only be appended. Every byte the model sent
//!    keeps the offset it was parsed at, and only invented bytes live past
//!    `source.len()`.
//!
//! 2. **Nothing is ever deleted.** Streamdown drops an incomplete image and an
//!    incomplete HTML tag. We cannot, and would not want to: the promise is
//!    that every character the model sent is on screen. Where we cannot
//!    complete a construct we leave it exactly as it came, rendering as its own
//!    literal text, which is the honest thing for it to look like.
//!
//! Neither of those makes the repair safe on its own. What makes it safe is
//! that the caller turns it off when the stream ends: the last render of a
//! message parses the raw source, so a `**` the model meant literally is a `**`
//! the reader ends up seeing. See `Markdown::finish_streaming`.

/// Not whitespace and not punctuation to CommonMark, so a delimiter appended
/// after it is still right-flanking and can therefore close.
///
/// Needed because the source often pauses on a space — `**Searching the ` —
/// and `**Searching the **` does not close, per the flanking rules. Appending
/// `\u{200B}**` does, and the character itself draws nothing.
const ZERO_WIDTH_SPACE: char = '\u{200B}';

/// The markers to append to `source` so that the construct it is in the middle
/// of renders as that construct.
///
/// Empty when there is nothing in flight, when the tail is somewhere a repair
/// would be wrong (inside a fence, an indented code block, an HTML block), or
/// when the construct is one we decline to complete.
pub fn completion_suffix(source: &str) -> String {
    let Some(tail) = tail_block(source) else {
        return String::new();
    };

    if let Some(completion) = table_completion(tail) {
        return completion;
    }

    let mut suffix = String::new();
    if ends_with_setext_underline(tail) {
        suffix.push(ZERO_WIDTH_SPACE);
    }

    let closers = inline_closers(inline_region(tail));
    if closers.is_empty() {
        return suffix;
    }

    if suffix.is_empty()
        && closers.starts_with(['*', '_', '~'])
        && tail.ends_with(char::is_whitespace)
    {
        suffix.push(ZERO_WIDTH_SPACE);
    }
    suffix.push_str(&closers);
    suffix
}

/// The last leaf block of `source`, which is the only place an unfinished
/// construct can be: emphasis does not survive a blank line, so anything left
/// open in an earlier block is already resolved and is meant literally.
///
/// `None` when the tail is not somewhere text is being written — inside an
/// unterminated fence, inside an indented code block, inside an HTML block —
/// because in all three the source is already being shown verbatim on purpose.
fn tail_block(source: &str) -> Option<&str> {
    let mut fence: Option<(u8, usize)> = None;
    let mut block_start = 0;
    let mut offset = 0;

    for line in source.split_inclusive('\n') {
        offset += line.len();
        let content = line.trim_end_matches(['\n', '\r']);

        if let Some((fence_char, fence_len)) = fence {
            if let Some((char, len, rest)) = fence_marker(content)
                && char == fence_char
                && len >= fence_len
                && rest.trim().is_empty()
            {
                fence = None;
                block_start = offset;
            }
            continue;
        }

        if let Some((char, len, _)) = fence_marker(content) {
            fence = Some((char, len));
            continue;
        }

        if content.trim().is_empty() {
            block_start = offset;
        }
    }

    if fence.is_some() {
        return None;
    }

    let tail = source.get(block_start..)?;
    if tail.trim().is_empty() {
        return None;
    }

    let first_line = tail.lines().next()?;
    if first_line.starts_with("    ") || first_line.starts_with('\t') {
        return None;
    }
    if first_line.trim_start().starts_with('<') {
        return None;
    }

    Some(tail)
}

/// `(fence character, run length, info string)` when `line` opens or closes a
/// fenced code block.
fn fence_marker(line: &str) -> Option<(u8, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() >= 4 {
        return None;
    }
    let char = *trimmed.as_bytes().first()?;
    if char != b'`' && char != b'~' {
        return None;
    }
    let len = trimmed.bytes().take_while(|byte| *byte == char).count();
    if len < 3 {
        return None;
    }
    let rest = trimmed.get(len..)?;
    // An info string on a backtick fence may not itself contain a backtick,
    // which is what keeps `` ```a``b `` from reading as a fence.
    if char == b'`' && rest.contains('`') {
        return None;
    }
    Some((char, len, rest))
}

/// Whether the tail ends on a line that a parser must read as a setext
/// underline, turning the paragraph above it into a heading.
///
/// This is the one case where the *absence* of a marker is the problem: a list
/// arriving under a paragraph passes through `-` on its way to `- item`, and
/// for those bytes the paragraph above jumps to heading size and back.
/// Streamdown breaks the pattern with a zero-width space and so do we.
fn ends_with_setext_underline(tail: &str) -> bool {
    if tail.ends_with('\n') {
        return false;
    }
    let mut lines = tail.rsplit('\n');
    let last = lines.next().unwrap_or_default().trim();
    if !matches!(last, "-" | "--" | "=" | "==") {
        return false;
    }
    let Some(previous) = lines.next() else {
        return false;
    };
    let previous = previous.trim();
    // Only a paragraph can be underlined. Under a list item the same `-` is the
    // next bullet, and breaking it would merge two items into one.
    !previous.is_empty() && !previous.starts_with('#') && !starts_list_item(previous)
}

fn starts_list_item(line: &str) -> bool {
    let mut chars = line.chars();
    match chars.next() {
        Some('-' | '*' | '+') => chars.next().is_none_or(|char| char == ' '),
        Some(char) if char.is_ascii_digit() => {
            let rest = line.trim_start_matches(|char: char| char.is_ascii_digit());
            matches!(rest.as_bytes().first(), Some(b'.' | b')'))
        }
        _ => false,
    }
}

/// The span of the tail whose inline delimiters are still in play.
///
/// Normally the whole tail. In a table it is the last cell only: emphasis does
/// not cross a cell boundary, so closing a `*` from the first cell at the end
/// of the last row would put a stray asterisk in the wrong place.
fn inline_region(tail: &str) -> &str {
    let is_table = tail.lines().nth(1).is_some_and(is_delimiter_row);
    if !is_table {
        return tail;
    }
    let last_line = tail.rsplit('\n').next().unwrap_or(tail);
    match last_line.rfind('|') {
        Some(position) => last_line.get(position + 1..).unwrap_or_default(),
        None => last_line,
    }
}

fn is_delimiter_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|')
        && trimmed.contains('-')
        && trimmed
            .chars()
            .all(|char| matches!(char, '|' | '-' | ':' | ' '))
}

#[derive(Clone, Copy)]
struct OpenRun {
    char: u8,
    len: usize,
}

/// The closing delimiters for everything left open in `region`, innermost
/// first, so that appending the result closes them in the right order.
fn inline_closers(region: &str) -> String {
    // A backtick run only opens a code span if a run of the same length turns
    // up later; otherwise it is literal text, and the emphasis after it still
    // counts. So the unmatched run is found first, and only the text before it
    // is scanned for delimiters — but only when the run is one we are actually
    // going to close. `**bold `**` has an unmatched backtick whose "content"
    // is nothing but the closing markers, which is how we can tell it is
    // literal rather than a span in flight.
    let open_span = open_code_span(region).filter(|(len, opener)| {
        region
            .get(opener + len..)
            .is_some_and(|content| content.chars().any(is_code_span_content))
    });
    let scan_end = open_span.map_or(region.len(), |(_, opener)| opener);

    let bytes = region.get(..scan_end).unwrap_or_default().as_bytes();
    let mut open: Vec<OpenRun> = Vec::new();
    let mut brackets: Vec<usize> = Vec::new();
    let mut url: Option<bool> = None;
    let mut trailing_literal_run = false;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];

        if byte == b'\\' {
            index += 2;
            continue;
        }

        if byte == b'`' {
            let len = run_length(bytes, index, b'`');
            index = matching_backtick_run(bytes, index + len, len).unwrap_or(index + len);
            continue;
        }

        if url.is_some() {
            // A link destination cannot contain a line ending, so a newline
            // means this was never a link and the state has to be abandoned.
            if byte == b')' || byte == b'\n' {
                url = None;
            }
            index += 1;
            continue;
        }

        match byte {
            b'[' => {
                brackets.push(index);
                index += 1;
            }
            b']' => {
                let opened = brackets.pop();
                if bytes.get(index + 1) == Some(&b'(') {
                    url = Some(opened.is_some_and(|start| start > 0 && bytes[start - 1] == b'!'));
                    index += 2;
                } else {
                    index += 1;
                }
            }
            b'*' | b'_' | b'~' => {
                let len = run_length(bytes, index, byte);
                let before = region
                    .get(..index)
                    .and_then(|text| text.chars().next_back());
                let after = region
                    .get(index + len..)
                    .and_then(|text| text.chars().next());
                let (can_open, can_close) = flanking(byte, before, after);
                // `~` is strikethrough only as a pair. A lone `~` opens a
                // subscript here, and completing that would eat the tilde out
                // of every `~/path` an agent prints. It may still *close*, so
                // that `~~struck~` finishes the closer it has started.
                let can_open = can_open && (byte != b'~' || len == 2);
                let mut remaining = len;

                if can_close {
                    while remaining > 0 {
                        let Some(position) = open.iter().rposition(|run| run.char == byte) else {
                            break;
                        };
                        // Everything opened after the run being closed can no
                        // longer match anything, exactly as CommonMark drops
                        // the delimiters between an opener and its closer.
                        open.truncate(position + 1);
                        let matched = remaining.min(open[position].len);
                        open[position].len -= matched;
                        remaining -= matched;
                        if open[position].len == 0 {
                            open.pop();
                        }
                    }
                }

                let opened = can_open && remaining > 0;
                if opened {
                    open.push(OpenRun {
                        char: byte,
                        len: remaining,
                    });
                }
                // A run at the very end that neither opened nor closed
                // anything is a delimiter whose meaning the next byte decides.
                // Appending to it would merge with it and change its length —
                // `**bold with *` plus `**` is a run of three, not a closer —
                // so nothing is appended for the one tick it takes to resolve.
                trailing_literal_run = index + len == bytes.len() && remaining == len && !opened;
                index += len;
            }
            _ => index += 1,
        }
    }

    if trailing_literal_run {
        return String::new();
    }

    let mut closers = String::new();

    if let Some(is_image) = url {
        // An image with half a destination would be rendered as a broken image
        // rather than as the text the model actually sent, so it is left alone
        // until the destination closes. Anything open around it is left alone
        // too: its closer would land inside the unfinished destination.
        if is_image {
            return String::new();
        }
        closers.push(')');
    } else if let Some((len, _)) = open_span {
        // A closing run has to be exactly as long as the opening one, so it
        // must not merge with backticks already at the end of the source.
        if region.ends_with('`') {
            closers.push(ZERO_WIDTH_SPACE);
        }
        for _ in 0..len {
            closers.push('`');
        }
    }

    for run in open.iter().rev() {
        for _ in 0..run.len {
            closers.push(run.char as char);
        }
    }

    closers
}

/// The first backtick run in `region` with no matching run after it, as
/// `(length, offset)`. Every earlier run is a complete code span and is skipped
/// whole.
fn open_code_span(region: &str) -> Option<(usize, usize)> {
    let bytes = region.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => {
                let len = run_length(bytes, index, b'`');
                match matching_backtick_run(bytes, index + len, len) {
                    Some(end) => index = end,
                    None => return Some((len, index)),
                }
            }
            _ => index += 1,
        }
    }
    None
}

/// Where the run of exactly `len` backticks that closes a span opened at `from`
/// ends, if there is one.
fn matching_backtick_run(bytes: &[u8], from: usize, len: usize) -> Option<usize> {
    let mut index = from;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => {
                let run = run_length(bytes, index, b'`');
                if run == len {
                    return Some(index + run);
                }
                index += run;
            }
            _ => index += 1,
        }
    }
    None
}

/// Whether a character is worth closing a code span for. Whitespace and bare
/// emphasis markers are not: they are what the text looks like when the
/// backtick was literal all along.
fn is_code_span_content(char: char) -> bool {
    !char.is_whitespace() && !matches!(char, '*' | '_' | '~' | '`')
}

fn run_length(bytes: &[u8], start: usize, char: u8) -> usize {
    bytes[start..]
        .iter()
        .take_while(|byte| **byte == char)
        .count()
}

/// CommonMark's left- and right-flanking rules, which decide whether a
/// delimiter run can open emphasis, close it, both, or neither.
///
/// These do most of the work of knowing what not to touch: the `*` in `2 * 3`
/// is flanked by whitespace and so opens nothing, the `_` in `snake_case` is
/// intraword and so does neither, and a `*` bullet is followed by a space.
fn flanking(char: u8, before: Option<char>, after: Option<char>) -> (bool, bool) {
    let before_whitespace = before.is_none_or(char::is_whitespace);
    let after_whitespace = after.is_none_or(char::is_whitespace);
    let before_punctuation = before.is_some_and(is_punctuation);
    let after_punctuation = after.is_some_and(is_punctuation);

    let left_flanking =
        !after_whitespace && (!after_punctuation || before_whitespace || before_punctuation);
    let right_flanking =
        !before_whitespace && (!before_punctuation || after_whitespace || after_punctuation);

    if char == b'_' {
        (
            left_flanking && (!right_flanking || before_punctuation),
            right_flanking && (!left_flanking || after_punctuation),
        )
    } else {
        (left_flanking, right_flanking)
    }
}

fn is_punctuation(char: char) -> bool {
    char.is_ascii_punctuation()
}

/// The delimiter row a table header is waiting for.
///
/// A header row renders as a paragraph of literal pipes until its delimiter
/// arrives, and the delimiter arrives a character at a time, so the raw-pipe
/// state is long rather than momentary. Synthesising the delimiter row from the
/// header's own column count ends it.
///
/// Only fires once the header row is newline-terminated. Firing on
/// `| Name | Size |` while it is still being written would build a table whose
/// column count changes under the reader as more cells arrive, which is a worse
/// thing to watch than the pipes.
fn table_completion(tail: &str) -> Option<String> {
    let mut lines = tail.split('\n');
    let columns = table_header_columns(lines.next()?)?;
    let partial = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    delimiter_completion(partial, columns)
}

fn table_header_columns(line: &str) -> Option<usize> {
    let line = line.trim_end();
    let indented = line.trim_start_matches(' ');
    if line.len() - indented.len() >= 4 {
        return None;
    }
    if !indented.starts_with('|') || !indented.ends_with('|') {
        return None;
    }
    let pipes = unescaped_pipes(indented);
    (pipes >= 2).then(|| pipes - 1)
}

fn unescaped_pipes(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'|' => {
                count += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    count
}

/// What to append to `partial` so it becomes a valid delimiter row of
/// `columns` cells. `None` when it already is one, or when it is not on its way
/// to being one.
fn delimiter_completion(partial: &str, columns: usize) -> Option<String> {
    let partial = partial.trim_end_matches('\r');
    if !partial
        .chars()
        .all(|char| matches!(char, '|' | '-' | ':' | ' '))
    {
        return None;
    }
    let trimmed = partial.trim_start_matches(' ');
    if partial.len() - trimmed.len() >= 4 {
        return None;
    }

    let mut completion = String::new();
    let (mut cells, in_progress) = if trimmed.is_empty() {
        completion.push('|');
        (0, "")
    } else if let Some(rest) = trimmed.strip_prefix('|') {
        (
            rest.matches('|').count(),
            rest.rsplit('|').next().unwrap_or_default(),
        )
    } else {
        return None;
    };

    if !in_progress.is_empty() {
        if in_progress.contains('-') {
            completion.push('|');
        } else {
            completion.push_str("---|");
        }
        cells += 1;
    }

    if cells > columns {
        return None;
    }
    for _ in cells..columns {
        completion.push_str("---|");
    }

    (!completion.is_empty()).then_some(completion)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point, in one line: the marker the owner watched arrive.
    #[test]
    fn an_unclosed_bold_run_is_closed() {
        assert_eq!(completion_suffix("**Searching"), "**");
        assert_eq!(completion_suffix("Now **searching for it"), "**");
    }

    #[test]
    fn a_closed_bold_run_is_left_alone() {
        assert_eq!(completion_suffix("**Searching**"), "");
        assert_eq!(completion_suffix("**Searching** and **more**"), "");
    }

    /// The first `*` of a closing `**` has arrived and the second has not.
    #[test]
    fn a_half_written_closing_run_is_finished() {
        assert_eq!(completion_suffix("**Searching*"), "*");
    }

    #[test]
    fn nested_emphasis_closes_innermost_first() {
        assert_eq!(completion_suffix("**bold and *italic"), "***");
        assert_eq!(completion_suffix("*italic and **bold"), "***");
        assert_eq!(completion_suffix("**bold and *italic* still bold"), "**");
    }

    #[test]
    fn underscore_emphasis_is_closed() {
        assert_eq!(completion_suffix("_italic"), "_");
        assert_eq!(completion_suffix("__bold"), "__");
    }

    /// The reason the flanking rules are implemented rather than a delimiter
    /// count: an identifier is not emphasis, and a count cannot tell.
    #[test]
    fn intraword_underscores_are_not_emphasis() {
        assert_eq!(completion_suffix("call snake_case_name here"), "");
        assert_eq!(completion_suffix("call snake_case_name"), "");
        assert_eq!(completion_suffix("a_b"), "");
    }

    #[test]
    fn arithmetic_and_bullets_are_not_emphasis() {
        assert_eq!(completion_suffix("2 * 3 = 6"), "");
        assert_eq!(completion_suffix("* first item"), "");
        assert_eq!(completion_suffix("- first\n* second"), "");
    }

    #[test]
    fn a_thematic_break_is_not_emphasis() {
        assert_eq!(completion_suffix("above\n\n***"), "");
        assert_eq!(completion_suffix("above\n\n---"), "");
    }

    #[test]
    fn a_bare_opening_run_with_nothing_after_it_is_left_alone() {
        assert_eq!(completion_suffix("**"), "");
        assert_eq!(completion_suffix("text **"), "");
        assert_eq!(completion_suffix("*"), "");
    }

    #[test]
    fn inline_code_is_closed_with_its_own_run_length() {
        assert_eq!(completion_suffix("run `cargo tes"), "`");
        assert_eq!(completion_suffix("run ``a ` b"), "``");
        assert_eq!(completion_suffix("run `cargo test`"), "");
    }

    /// Nothing inside a code span is markdown, so nothing inside it is repaired
    /// — and the span's own closer comes before any emphasis closer, because
    /// the emphasis was opened outside it.
    #[test]
    fn markers_inside_inline_code_are_not_completed() {
        assert_eq!(completion_suffix("`a ** b`"), "");
        assert_eq!(completion_suffix("**bold `a ** b"), "`**");
    }

    /// A backtick with nothing after it yet is not a span worth closing, and
    /// closing it would draw two literal backticks rather than code. Whatever
    /// it is wrapped in is still closed.
    #[test]
    fn a_backtick_with_no_content_yet_is_not_closed() {
        assert_eq!(completion_suffix("text `"), "");
        assert_eq!(completion_suffix("**bold `"), "**");
        assert_eq!(completion_suffix("**bold `code"), "`**");
    }

    /// The closing run has to be exactly as long as the opening one, so it must
    /// not merge with backticks already at the end: `` ``code` `` plus two
    /// backticks is a run of three and closes nothing.
    #[test]
    fn a_code_span_closer_does_not_merge_with_the_backticks_before_it() {
        assert_eq!(completion_suffix("run ``code`"), "\u{200B}``");
    }

    #[test]
    fn nothing_inside_an_open_fence_is_repaired() {
        assert_eq!(completion_suffix("```rust\nlet a = b * c"), "");
        assert_eq!(completion_suffix("```\n**not bold"), "");
        assert_eq!(completion_suffix("~~~\n**not bold"), "");
    }

    #[test]
    fn a_closed_fence_hands_the_tail_back() {
        assert_eq!(
            completion_suffix("```rust\nlet a = 1;\n```\n\n**Then"),
            "**"
        );
    }

    #[test]
    fn an_indented_code_block_is_left_alone() {
        assert_eq!(completion_suffix("para\n\n    let a = b * c"), "");
    }

    #[test]
    fn an_html_block_is_left_alone() {
        assert_eq!(completion_suffix("para\n\n<div class=\"a *b"), "");
    }

    /// Emphasis does not cross a blank line, so a marker in an earlier
    /// paragraph is already resolved and is meant literally.
    #[test]
    fn only_the_last_block_is_repaired() {
        assert_eq!(completion_suffix("**never closed\n\nnext paragraph"), "");
        assert_eq!(completion_suffix("**never closed\n\n**this one"), "**");
    }

    #[test]
    fn a_heading_is_repaired_like_any_other_block() {
        assert_eq!(completion_suffix("## A **bold"), "**");
    }

    #[test]
    fn a_link_destination_is_closed_but_never_invented() {
        assert_eq!(completion_suffix("see [Omega](https://ex"), ")");
        assert_eq!(completion_suffix("see [Omega](https://ex)"), "");
        // No `](` yet: nothing says this is a link, and `[` reads fine as
        // itself.
        assert_eq!(completion_suffix("see [Omega"), "");
        assert_eq!(completion_suffix("see [Omega]"), "");
    }

    #[test]
    fn an_image_with_half_a_destination_is_left_alone() {
        assert_eq!(completion_suffix("![alt](https://ex"), "");
    }

    #[test]
    fn emphasis_around_an_unfinished_link_closes_after_it() {
        assert_eq!(completion_suffix("**see [Omega](https://ex"), ")**");
    }

    #[test]
    fn strikethrough_pairs_are_closed_and_lone_tildes_are_not() {
        assert_eq!(completion_suffix("~~struck"), "~~");
        assert_eq!(completion_suffix("~~struck~~"), "");
        assert_eq!(completion_suffix("look in ~/work for it"), "");
        assert_eq!(completion_suffix("look in ~/work"), "");
    }

    /// `**bold ` cannot be closed by `**` — the run would be preceded by a
    /// space and so cannot close. The zero-width space is what makes it able
    /// to, without moving a byte the model sent.
    #[test]
    fn a_closer_after_trailing_whitespace_is_made_able_to_close() {
        assert_eq!(completion_suffix("**bold and "), "\u{200B}**");
        assert_eq!(completion_suffix("**bold and\n"), "\u{200B}**");
    }

    #[test]
    fn a_setext_underline_forming_under_a_paragraph_is_broken() {
        assert_eq!(completion_suffix("Some text\n-"), "\u{200B}");
        assert_eq!(completion_suffix("Some text\n--"), "\u{200B}");
        assert_eq!(completion_suffix("Some text\n="), "\u{200B}");
        // Under a list item the same `-` is the next bullet.
        assert_eq!(completion_suffix("- one\n-"), "");
        // Three is a thematic break the model meant.
        assert_eq!(completion_suffix("Some text\n---"), "");
    }

    #[test]
    fn a_setext_break_and_a_bold_closer_are_both_appended() {
        assert_eq!(completion_suffix("**bold\n-"), "\u{200B}**");
    }

    #[test]
    fn a_table_header_gets_the_delimiter_row_it_is_waiting_for() {
        assert_eq!(completion_suffix("| Name | Size |\n"), "|---|---|");
        assert_eq!(completion_suffix("| Name | Size |\n|"), "---|---|");
        assert_eq!(completion_suffix("| Name | Size |\n|--"), "|---|");
        assert_eq!(completion_suffix("| Name | Size |\n|---|"), "---|");
        assert_eq!(completion_suffix("| Name | Size |\n|---|---|"), "");
        assert_eq!(completion_suffix("| A |\n"), "|---|");
    }

    #[test]
    fn an_unterminated_table_header_waits_for_its_newline() {
        assert_eq!(completion_suffix("| Name | Si"), "");
        assert_eq!(completion_suffix("| Name | Size |"), "");
    }

    #[test]
    fn a_paragraph_that_merely_contains_a_pipe_is_not_a_table() {
        assert_eq!(completion_suffix("run a | b here\n"), "");
        assert_eq!(completion_suffix("| not a row\n"), "");
    }

    /// Inside a table, emphasis belongs to one cell. Closing a run from an
    /// earlier cell at the end of the last row would put the asterisks in the
    /// wrong cell entirely.
    #[test]
    fn emphasis_in_a_table_is_closed_within_its_own_cell() {
        assert_eq!(completion_suffix("| a | b |\n|---|---|\n| *one | two"), "");
        assert_eq!(completion_suffix("| a | b |\n|---|---|\n| one | *two"), "*");
    }

    /// The source is sitting on a delimiter that has not decided what it is.
    /// Appending to it would merge with it and change its run length, so the
    /// repair waits the one byte it takes to find out.
    #[test]
    fn a_delimiter_whose_meaning_the_next_byte_decides_is_left_alone() {
        assert_eq!(completion_suffix("**bold with *"), "");
        assert_eq!(completion_suffix("__bold with _"), "");
        assert_eq!(completion_suffix("**bold with *n"), "***");
    }

    #[test]
    fn escaped_markers_are_not_delimiters() {
        assert_eq!(completion_suffix("a \\*b"), "");
        assert_eq!(completion_suffix("a \\`b"), "");
    }

    #[test]
    fn empty_and_whitespace_sources_are_left_alone() {
        assert_eq!(completion_suffix(""), "");
        assert_eq!(completion_suffix("   \n\n"), "");
    }

    /// The invariant that makes the whole thing safe to append: whatever we
    /// return, the source itself is untouched and only grows at the end. Fed a
    /// document one byte at a time, no prefix may ever produce a suffix that
    /// starts by deleting or rewriting.
    ///
    /// Byte-at-a-time is also how this class of bug is actually found: the
    /// failures are all at token boundaries nobody thought to write down.
    #[test]
    fn every_prefix_of_a_document_completes_to_something_balanced() {
        for document in DOCUMENTS {
            for end in 0..=document.len() {
                if !document.is_char_boundary(end) {
                    continue;
                }
                let prefix = &document[..end];
                let suffix = completion_suffix(prefix);
                let completed = format!("{prefix}{suffix}");
                assert!(
                    completed.starts_with(prefix),
                    "the repair for {prefix:?} was not a suffix"
                );
                assert!(
                    completion_suffix(&completed).is_empty(),
                    "the repair for {prefix:?} was {suffix:?}, which is itself \
                     still incomplete"
                );
            }
        }
    }

    /// Documents chosen for their token boundaries rather than their prose:
    /// every construct in scope, plus the text that must survive *not* being
    /// treated as a construct.
    const DOCUMENTS: &[&str] = &[
        "# Title\n\nA **bold** claim with `code` and *stress*, \
         plus a [link](https://example.com) and ~~a strike~~.\n\n\
         - one\n- two with **bold**\n\n\
         | Name | Size |\n|---|---|\n| a | 1 |\n| b | *2* |\n\n\
         ```rust\nlet a = b * c;\n```\n\n\
         Trailing paragraph with snake_case_name and ~/work in it.\n",
        "Look in ~/work and run `cargo test -p markdown` first. \
         2 * 3 = 6, and a_b_c is one identifier.\n\n\
         The literal ** stays, and so does a lone * on its own.\n",
        "### **Searching** for it\n\nThen ***both at once*** and \
         **bold with *nested* inside** and `a ** b` in code.\n",
        "Intro paragraph.\n- first\n- second\n\n> quoted **bold** text\n\n\
         1. one\n2. two\n",
        "See ![diagram](https://example.com/a.png) and [text][ref] and \
         [another](https://example.com/b_c_d).\n",
        "| A | B | C |\n|:--|:-:|--:|\n| **x** | `y` | z |\n",
        "```\n**not bold** and `not code`\n```\n\nAfter the fence.\n",
    ];

    /// Every byte survives to the finished document. Nothing here removes
    /// anything, and the finished source is what the reader ends up parsing.
    #[test]
    fn the_finished_document_needs_no_repair() {
        let document = "**bold** and `code` and [link](https://example.com)\n";
        assert_eq!(completion_suffix(document), "");
    }
}
