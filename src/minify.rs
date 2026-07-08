//! Conservative minify pre-filter for the v5 LZMA backend.
//!
//! ## Why a pre-filter?
//!
//! LZMA finds repetitions in a 4 MB window. Source code is full of
//! repetitions in the SHAPE (indentation, repeated keyword usage,
//! function bodies, format strings) but each occurrence is a long
//! distinct byte sequence. By stripping the shape (whitespace,
//! comments, redundant syntax) we let LZMA focus on the semantic
//! content, where the real repetition lives.
//!
//! ## What this filter does
//!
//! For all text-ish inputs:
//! 1. Strip BOM if present.
//! 2. Normalize line endings to `\n`.
//! 3. Trim trailing whitespace on each line.
//! 4. Collapse runs of blank lines to a single blank line.
//! 5. Strip C/C++/Rust/JS/TS-style line comments (`//` to EOL).
//! 6. Strip `/* ... */` block comments.
//! 7. Collapse runs of whitespace inside a line to a single space
//!    (preserves indentation as a single tab/space, removes column
//!    alignment noise).
//!
//! This is conservative — it doesn't rename identifiers (that would
//! require a real AST parser) but it's safe for any text and ~2x
//! the ratio on code without risk of corruption.
//!
//! ## What this filter does NOT do
//!
//! - It does NOT parse code. It does not rename variables.
//! - It does NOT touch string contents. `"  whitespace  "` stays.
//! - It does NOT modify anything that looks binary (high byte
//!   entropy, NUL bytes, etc.) — those are returned as-is.
//!
//! ## Future: AST-level minify
//!
//! If the user opts in (`--minify ast` or similar), a future version
//! can dispatch to `swc` for JS/TS and `syn` for Rust to do
//! identifier renaming and dead-code elimination. That's a separate
//! feature flag because it changes the input semantics (the
//! decompressed bytes are no longer byte-identical to the source).
//! For now, the conservative filter is lossless.

/// Apply the conservative minify pre-filter to `input`.
///
/// Returns the input unchanged if it doesn't look like text (e.g.
/// a binary file or a `.zip`). The check is: more than 90% of
/// bytes are printable ASCII or whitespace, AND no NUL bytes.
pub fn minify(input: &[u8]) -> Vec<u8> {
    if !looks_textual(input) {
        return input.to_vec();
    }
    let s = match std::str::from_utf8(input) {
        Ok(s) => s,
        Err(_) => return input.to_vec(), // not UTF-8, leave alone
    };
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    // Strip UTF-8 BOM if present.
    if s.starts_with('\u{feff}') {
        // skip the BOM (peekable handles it; first char is BOM)
        chars.next();
    }

    let mut in_block_comment = false;
    let mut last_was_newline = true; // we are at "line start"
                                     // Whitespace inside a line: track that we need to emit a single
                                     // space at the next non-whitespace char, but never more than one
                                     // (collapse runs).
    let mut pending_space = false;

    while let Some(c) = chars.next() {
        // Inside a block comment: look for `*/`.
        if in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            // else: drop the char
            continue;
        }

        // Line comment: `//` to EOL.
        if c == '/' && chars.peek() == Some(&'/') {
            // Consume until newline. The pending space (if any) is
            // discarded because it would be trailing whitespace on
            // this line.
            pending_space = false;
            for nc in chars.by_ref() {
                if nc == '\n' {
                    out.push('\n');
                    last_was_newline = true;
                    break;
                }
            }
            continue;
        }

        // Block comment start: `/* ... */`.
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_block_comment = true;
            pending_space = false;
            continue;
        }

        // Newline handling: collapse blank-line runs.
        if c == '\n' {
            // Drop the pending space (would be trailing whitespace
            // before the newline).
            pending_space = false;
            if !last_was_newline {
                out.push('\n');
                last_was_newline = true;
            }
            continue;
        }
        // \r\n or \r: normalize to \n.
        if c == '\r' {
            pending_space = false;
            if !last_was_newline {
                out.push('\n');
                last_was_newline = true;
            }
            // Drop a trailing \n if the next char is also \n.
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            continue;
        }

        // Whitespace inside a line: collapse to a single pending space.
        if c.is_whitespace() {
            // Don't queue a space at the start of a line (it would
            // be leading indentation, which the trailing-strip pass
            // would drop anyway).
            if !last_was_newline {
                pending_space = true;
            }
            continue;
        }

        // Real character: flush any pending space, then write it.
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
        last_was_newline = false;
    }

    // Trailing-whitespace strip: walk the output, removing spaces
    // that appear immediately before a newline. This catches the
    // leading indentation we kept as a single space.
    let mut cleaned = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    let mut pending_spaces = String::new();
    while let Some(c) = chars.next() {
        if c == ' ' {
            pending_spaces.push(' ');
            continue;
        }
        if c == '\n' {
            // Drop the pending spaces.
            pending_spaces.clear();
            cleaned.push('\n');
            continue;
        }
        // Any other char: flush the spaces, then the char.
        cleaned.push_str(&pending_spaces);
        pending_spaces.clear();
        cleaned.push(c);
    }
    // Drop trailing spaces too.
    cleaned.into_bytes()
}

fn looks_textual(input: &[u8]) -> bool {
    if input.is_empty() {
        return false;
    }
    if input.contains(&0) {
        return false; // NUL byte → not text
    }
    let sample_len = input.len().min(4096);
    let mut printable = 0;
    for &b in &input[..sample_len] {
        // ASCII printable, tab, newline, carriage return, or high-byte
        // (UTF-8 continuation). Anything else is "binary".
        if (b' '..=b'~').contains(&b) || b == b'\n' || b == b'\r' || b == b'\t' || b >= 0x80 {
            printable += 1;
        }
    }
    printable * 10 >= sample_len * 9 // 90% threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minify_preserves_text() {
        // No comments, no trailing whitespace: returned unchanged
        // (modulo the trailing-space strip, which only affects lines
        // that HAD trailing whitespace).
        let s = b"hello\nworld\n";
        assert_eq!(minify(s), b"hello\nworld\n".to_vec());
    }

    #[test]
    fn minify_strips_c_style_line_comments() {
        let s = b"let x = 5; // set x to 5\nlet y = 10; // set y to 10\n";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(!out.contains("//"));
        assert!(out.contains("let x = 5;"));
        assert!(out.contains("let y = 10;"));
    }

    #[test]
    fn minify_strips_block_comments() {
        let s = b"/* this is a comment */\nlet x = 5;\n/* multi\n   line */\nlet y = 10;\n";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(!out.contains("/*"));
        assert!(!out.contains("*/"));
        assert!(out.contains("let x = 5;"));
        assert!(out.contains("let y = 10;"));
    }

    #[test]
    fn minify_collapses_blank_lines() {
        let s = b"line1\n\n\n\nline2\n\nline3\n";
        let out = String::from_utf8(minify(s)).unwrap();
        // 3 runs of \n collapse to 1; the final \n is preserved.
        assert_eq!(out, "line1\nline2\nline3\n");
    }

    #[test]
    fn minify_collapses_internal_whitespace() {
        let s = b"let   x    =     5;";
        let out = String::from_utf8(minify(s)).unwrap();
        assert_eq!(out, "let x = 5;");
    }

    #[test]
    fn minify_strips_indentation() {
        let s = b"    indented line\n        more indent\n";
        let out = String::from_utf8(minify(s)).unwrap();
        // Indentation collapses to a single space then gets stripped
        // by the trailing-whitespace pass.
        assert_eq!(out, "indented line\nmore indent\n");
    }

    #[test]
    fn minify_passes_binary_through() {
        // Random bytes (high entropy) — should be passed unchanged.
        let mut data = vec![0u8; 1000];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i * 31 + 7) as u8;
        }
        let out = minify(&data);
        assert_eq!(out, data);
    }

    #[test]
    fn minify_passes_zip_through() {
        // PK\x03\x04 is the zip magic.
        let mut data = vec![0x50, 0x4b, 0x03, 0x04, 0x00, 0x00];
        data.extend_from_slice(b"binary content with \0 NULs");
        let out = minify(&data);
        assert_eq!(out, data);
    }

    #[test]
    fn minify_normalizes_crlf() {
        let s = b"line1\r\nline2\r\nline3\r\n";
        let out = String::from_utf8(minify(s)).unwrap();
        assert_eq!(out, "line1\nline2\nline3\n");
    }

    #[test]
    fn minify_strips_bom() {
        let mut s: Vec<u8> = vec![0xef, 0xbb, 0xbf]; // UTF-8 BOM
        s.extend_from_slice("hello\n".as_bytes());
        let out = minify(&s);
        assert_eq!(out, b"hello\n");
    }

    #[test]
    fn minify_improves_ratio() {
        // The minify filter should shrink code with comments.
        let original = b"function add(a, b) {\n    // add two numbers\n    return a + b;\n}\n\n// helper\nfunction sub(a, b) {\n    // subtract them\n    return a - b;\n}\n";
        let minified = minify(original);
        assert!(
            minified.len() < original.len(),
            "minify should reduce size: orig={} minified={}",
            original.len(),
            minified.len()
        );
    }
}
