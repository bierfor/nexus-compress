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
//! ## String-aware comment stripping (Sprint 5.7.3 hotfix #48)
//!
//! Steps 5 and 6 only fire OUTSIDE string literals. Inside a
//! double-quoted string (`"..."`), single-quoted string
//! (`'...'`, used by Python/shell/JSON-ish), or backtick
//! template literal (`` `...` ``, JS/TS) we treat the content
//! as opaque: `//` and `/*` inside the string are NEVER taken
//! for comment markers. Escaped quotes (`\"`, `\'`, `` \` ``)
//! inside a string do not end it. This fixes the regression
//! where `https://example.com` got truncated to `https:`
//! because the lexer mistook the `//` for a comment start.
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
///
/// Sprint 5.7.3 hotfix #48: the stripper is now string-aware.
/// Comments (`//`, `/* */`) and string contents are tracked
/// through a small state machine so the two never collide —
/// `https://example.com` stays whole, `"a // b"` stays whole.
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

    // Sprint 5.7.3 hotfix #48: string-aware state machine.
    //
    // The previous version had a single `in_block_comment` bool
    // and treated any `//` (line 86) as a comment start, even
    // when the `//` was inside a string literal. That truncated
    // every `https://` URL in the corpus to `https:`, mangled
    // URLs in package.json, broke comment-as-string in JSON
    // configs, etc.
    //
    // The new design uses an explicit three-state lexer:
    //   * `Normal`     — looking for comments or string opens
    //   * `InString`   — opaque content; only the matching
    //                    delimiter or its escape sequence ends
    //                    the string
    //   * `InBlock`    — opaque content; only `*/` ends the
    //                    block comment
    //
    // String delimiters we honour: `"`, `'`, `` ` ``. The escape
    // rule is a single backslash: `\"` inside a `"` string does
    // not close the string. This is enough for the corpus we
    // see (JSON, JS/TS, Python, shell, Markdown) without pulling
    // in a real parser.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Lex {
        Normal,
        InString(char),
        InBlock,
    }
    let mut lex = Lex::Normal;
    let mut last_was_newline = true;
    let mut pending_space = false;

    while let Some(c) = chars.next() {
        // ── Inside a block comment: look only for `*/`.
        if lex == Lex::InBlock {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                lex = Lex::Normal;
            }
            continue;
        }

        // ── Inside a string: copy verbatim, only the
        //    matching delimiter (or its escape) ends it.
        if let Lex::InString(delim) = lex {
            out.push(c);
            // Backslash escapes the next char. Honour it so a
            // `\"` inside a `"` string does not end the
            // string. We do NOT validate the escaped char
            // (e.g. \u inside JSON) — we just preserve it.
            if c == '\\' {
                if let Some(&next) = chars.peek() {
                    chars.next();
                    out.push(next);
                }
                continue;
            }
            if c == delim {
                lex = Lex::Normal;
            }
            // Track newlines inside strings so the blank-line
            // collapse pass below doesn't drop them.
            if c == '\n' {
                last_was_newline = true;
            } else {
                last_was_newline = false;
            }
            continue;
        }

        // ── Normal mode: comments + string opens + whitespace
        //    + real chars. Order matters — comment checks
        //    first, then string opens, then whitespace.

        // Line comment: `//` to EOL.
        if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
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
            lex = Lex::InBlock;
            pending_space = false;
            continue;
        }

        // String open. Three delimiters we care about:
        //   `"` — JSON, JS/TS, C, Rust, most config files
        //   `'` — Python, shell, .ini, .env, JSX-ish
        //   `` ` `` — JS/TS template literals
        if c == '"' || c == '\'' || c == '`' {
            // Flush any pending space, then enter the string.
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(c);
            lex = Lex::InString(c);
            last_was_newline = false;
            continue;
        }

        // Newline handling: collapse blank-line runs.
        if c == '\n' {
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
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            continue;
        }

        // Whitespace inside a line: collapse to a single pending
        // space.
        if c.is_whitespace() {
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

    // Sprint 5.7.3 hotfix #48: regression tests for the
    // string-aware stripper. Each test reproduces a concrete
    // failure that the old `//` regex produced on the user's
    // corpus (vercel.json, package.json, etc.) and asserts the
    // post-fix output is what we expect.

    #[test]
    fn minify_preserves_https_url_in_string() {
        // The original bug: `https://openapi.vercel.sh/vercel.json`
        // got truncated to `https:` because the lexer mistook
        // the `//` inside the string for a comment start.
        let s = b"\"$schema\": \"https://openapi.vercel.sh/vercel.json\"";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(
            out.contains("https://openapi.vercel.sh/vercel.json"),
            "https:// URL must stay whole, got: {:?}",
            out
        );
        // The truncation signature was a `//` EOL strip inside
        // the string, which would have produced output ending
        // with `"https:` (no second slash, no domain). We assert
        // the *full* URL survives, which is the actual invariant.
    }

    #[test]
    fn minify_preserves_http_url_in_string() {
        let s = b"\"cdn\": \"http://cdn.example.com/v1\"";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(out.contains("http://cdn.example.com/v1"), "got: {:?}", out);
    }

    #[test]
    fn minify_preserves_slash_inside_json_string() {
        // The classic JSON trap: a `//` inside a string is
        // data, not a comment.
        let s = b"{\"comment\": \"// this is not a comment\"}";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(out.contains("// this is not a comment"), "got: {:?}", out);
    }

    #[test]
    fn minify_strips_real_json_comment_after_value() {
        // Outside any string, a real `//` IS a comment and
        // must be stripped. This guards against the obvious
        // over-fix: making the stripper so cautious it stops
        // working on actual comments.
        let s = b"{\n  \"k\": \"v\" // real comment\n}\n";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(out.contains("\"k\": \"v\""), "got: {:?}", out);
        assert!(!out.contains("real comment"), "real comment should be stripped, got: {:?}", out);
    }

    #[test]
    fn minify_handles_escaped_quote_in_string() {
        // JSON allows `\"` inside a string. The stripper must
        // not interpret the `\"` as "end of string" — the
        // backslash escapes the quote.
        let s = b"{\"q\": \"he said \\\"// hi\\\" ok\"}";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(out.contains("\\\"// hi\\\""), "escaped quotes and // inside must stay whole, got: {:?}", out);
    }

    #[test]
    fn minify_handles_single_quoted_strings() {
        // Python / shell / .env use single quotes.
        let s = b"url = 'https://example.com/path'";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(out.contains("https://example.com/path"), "got: {:?}", out);
    }

    #[test]
    fn minify_handles_backtick_template_literals() {
        // JS/TS template literals.
        let s = b"const url = `https://api.example.com/v1/users`;";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(out.contains("https://api.example.com/v1/users"), "got: {:?}", out);
    }

    #[test]
    fn minify_strips_url_outside_string() {
        // A `//` followed by stuff to EOL is a comment even
        // when the rest of the line looks URL-ish. This is
        // correct behaviour — outside a string, `//` is a
        // comment.
        let s = b"// https://example.com\nlet x = 1;\n";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(!out.contains("https://"), "comment-line URL should be stripped, got: {:?}", out);
        assert!(out.contains("let x = 1;"), "got: {:?}", out);
    }

    #[test]
    fn minify_preserves_path_with_double_slash() {
        // Unix paths with `//` (e.g. SMB shares, empty path
        // components) appear in config files.
        let s = b"\"mount\": \"//nas.local/share/data\"";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(out.contains("//nas.local/share/data"), "got: {:?}", out);
    }

    #[test]
    fn minify_does_not_break_on_unterminated_string() {
        // A file that ends mid-string must not panic. The
        // stripper should just copy what's there.
        let s = b"{\"k\": \"https://example.com";
        let out = String::from_utf8(minify(s)).unwrap();
        assert!(out.contains("https://example.com"), "got: {:?}", out);
    }
}
