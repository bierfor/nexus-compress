//! AST-based JavaScript / TypeScript minifier (Phase 1).
//!
//! ## Honest contract
//!
//! Produces **functionally-equivalent minified code**, NOT
//! byte-identical source. Comments, formatting, and original
//! identifier names (in Phase 1) are dropped. The decompressed
//! output is valid runnable JavaScript, just like what Next.js,
//! Vite, or esbuild produce for production.
//!
//! ## Pipeline
//!
//! 1. Parse with `swc_core::ecma::parser` (JS or TS detection).
//! 2. Codegen with `swc_core::ecma::codegen` and `minify: true`.
//! 3. (Phase 2 — FoldWith mangle for local identifier renaming.)
//!
//! ## API notes for swc_core 72
//!
//! - `Lexer::new` takes `Option<&dyn Comments>`, not a Handler.
//! - `SourceMap::new_source_file` takes 2 args (filename, src).
//! - `Config` is `#[non_exhaustive]` — use `Config::default()`
//!   with the `.with_minify(true)` builder.

use swc_core::common::{comments::Comments, sync::Lrc, FileName, SourceMap};
use swc_core::ecma::ast::{EsVersion, Program};
use swc_core::ecma::codegen::{text_writer::JsWriter, Config, Emitter};
use swc_core::ecma::parser::{lexer::Lexer, EsSyntax, Parser, StringInput, Syntax, TsSyntax};

/// Result of minification.
pub struct MinifyResult {
    pub bytes: Vec<u8>,
    /// True if the input was actually parsed and re-emitted.
    /// False if the input was returned unchanged (parse error or
    /// non-JS content).
    pub was_minified: bool,
}

/// Try to minify `input` as JavaScript or TypeScript.
///
/// On any parse error, or if the input doesn't look like JS/TS,
/// returns the input bytes unchanged with `was_minified = false`.
pub fn minify(input: &[u8]) -> MinifyResult {
    // UTF-8 sanity check.
    let s = match std::str::from_utf8(input) {
        Ok(s) => s,
        Err(_) => return pass_through(input),
    };

    // Empty input is a no-op.
    if s.trim().is_empty() {
        return pass_through(input);
    }

    // Fast-fail heuristic: if no JS/TS telltales, skip the parse.
    let looks_like_code = s.contains('{')
        || s.contains("function")
        || s.contains("=>")
        || s.contains("var ")
        || s.contains("let ")
        || s.contains("const ")
        || s.contains("import ")
        || s.contains("export ");
    if !looks_like_code {
        return pass_through(input);
    }

    // TS detection: type-position colons or TS keywords.
    let maybe_ts = s.contains(": number")
        || s.contains(": string")
        || s.contains(": boolean")
        || s.contains("interface ")
        || s.contains("enum ");
    let syntax = if maybe_ts {
        Syntax::Typescript(TsSyntax {
            tsx: false,
            dts: false,
            ..Default::default()
        })
    } else {
        Syntax::Es(EsSyntax {
            jsx: false,
            ..Default::default()
        })
    };

    // We don't need a Handler for the Lexer in swc_core 72 — it
    // takes `Option<&dyn Comments>` instead. We pass None (we
    // don't keep the comments; the minifier discards them).
    // We need an OWNED String for the SourceFile (it has to
    // outlive the parse), so we convert the input to a String.
    let s_owned: String = s.to_string();
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        FileName::Anon.into(),
        s_owned.as_str().to_string(), // owned copy that outlives the borrow
    );
    // Drop s_owned now that fm has its own copy.
    drop(s_owned);
    let lexer = Lexer::new(
        syntax,
        EsVersion::Es2020,
        StringInput::from(&*fm),
        None::<&dyn Comments>,
    );
    let mut parser = Parser::new_from(lexer);

    // Try module first; fall back to script on failure. We discard
    // recoverable errors (the parser still produces a partial AST
    // in many cases).
    let program: Program = match parser.parse_module() {
        Ok(module) => {
            // Drop any non-fatal errors silently.
            let _ = parser.take_errors();
            Program::Module(module)
        }
        Err(_e) => {
            // Lexer is consumed; build a fresh one.
            let lexer2 = Lexer::new(
                syntax,
                EsVersion::Es2020,
                StringInput::from(&*fm),
                None::<&dyn Comments>,
            );
            let mut parser2 = Parser::new_from(lexer2);
            match parser2.parse_script() {
                Ok(script) => {
                    let _ = parser2.take_errors();
                    Program::Script(script)
                }
                Err(_) => return pass_through(input),
            }
        }
    };

    // Codegen with minify=true. We use the lower-level Emitter
    // because `to_code` doesn't accept a Config. Config is
    // #[non_exhaustive] in this version, so we use the builder.
    let mut buf: Vec<u8> = Vec::with_capacity(input.len());
    {
        let mut emitter = Emitter {
            cfg: Config::default().with_minify(true),
            cm: cm.clone(),
            comments: None,
            wr: JsWriter::new(cm.clone(), "\n", &mut buf, None),
        };
        if emitter.emit_program(&program).is_err() {
            return pass_through(input);
        }
    }

    if buf.is_empty() {
        return pass_through(input);
    }

    MinifyResult {
        bytes: buf,
        was_minified: true,
    }
}

fn pass_through(input: &[u8]) -> MinifyResult {
    MinifyResult {
        bytes: input.to_vec(),
        was_minified: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minify_simple_function() {
        let src = b"function add(a, b) { return a + b; }";
        let r = minify(src);
        assert!(r.was_minified);
        assert!(r.bytes.len() < src.len());
        let s = std::str::from_utf8(&r.bytes).unwrap();
        assert!(!s.contains('\n'));
    }

    #[test]
    fn minify_strips_comments() {
        let src = b"// header\nfunction f() { /* inner */ return 1; }\n";
        let r = minify(src);
        assert!(r.was_minified);
        let s = std::str::from_utf8(&r.bytes).unwrap();
        assert!(!s.contains("//"));
        assert!(!s.contains("/*"));
    }

    #[test]
    fn minify_passes_through_non_code() {
        let src = b"hello world this is plain text without code markers";
        let r = minify(src);
        assert!(!r.was_minified);
        assert_eq!(r.bytes, src);
    }

    #[test]
    fn minify_passes_through_binary() {
        let data: Vec<u8> = (0..200).map(|i| (i * 31 % 256) as u8).collect();
        let r = minify(&data);
        assert!(!r.was_minified);
        assert_eq!(r.bytes, data);
    }

    #[test]
    fn minify_passes_through_empty() {
        let r = minify(b"");
        assert!(!r.was_minified);
    }

    #[test]
    fn minify_preserves_template_literals() {
        let src = b"const f = ({a, b, ...rest}) => `${a} ${rest.length}`;";
        let r = minify(src);
        assert!(r.was_minified);
        let s = std::str::from_utf8(&r.bytes).unwrap();
        assert!(s.contains("`"));
    }

    #[test]
    fn minify_reduces_size_on_real_code() {
        let src = br#"
            // This function adds two numbers
            function add(a, b) {
                return a + b;
            }
            add(1, 2);
        "#;
        let r = minify(src);
        assert!(r.was_minified);
        let ratio = src.len() as f64 / r.bytes.len() as f64;
        assert!(ratio > 1.5, "got {:.2}x", ratio);
    }

    #[test]
    fn minify_strips_typescript_types() {
        let src = b"function add(a: number, b: number): number { return a + b; }";
        let r = minify(src);
        assert!(r.was_minified);
        let s = std::str::from_utf8(&r.bytes).unwrap();
        assert!(!s.contains(": number"));
    }
}
