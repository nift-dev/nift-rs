//! `minify` - native Rust multi-format minifier (Minify++ behavioural
//! contract).
//!
//! Standalone, independently useful: `format_for_extension` +
//! `minify(Format, &str)` over HTML, CSS, JavaScript, JSX, JSON, XML and SVG.
//!
//! The algorithms are ported from Minify++ (the product contract) into
//! idiomatic Rust: byte/char scanning over owned strings, no C++ buffers.
//! JavaScript keeps every significant newline (ASI-safe); CSS/HTML/XML handle
//! the token-boundary edge cases (comment openers, math operators, strings);
//! JSX preserves JSX boundaries.

/// Minification format (Minify++ `Format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Html,
    Css,
    JavaScript,
    Jsx,
    Json,
    Xml,
    Svg,
}

/// Map a file extension (with or without leading dot, case-insensitive) to a
/// format (Minify++ `format_for_extension`).
pub fn format_for_extension(extension: &str) -> Option<Format> {
    let mut ext = extension.to_ascii_lowercase();
    if !ext.is_empty() && !ext.starts_with('.') {
        ext.insert(0, '.');
    }
    match ext.as_str() {
        ".html" | ".htm" => Some(Format::Html),
        ".css" => Some(Format::Css),
        ".js" | ".mjs" | ".cjs" => Some(Format::JavaScript),
        ".jsx" => Some(Format::Jsx),
        ".json" => Some(Format::Json),
        ".xml" => Some(Format::Xml),
        ".svg" => Some(Format::Svg),
        _ => None,
    }
}

/// Minify `input` in `format`. Returns the minified text, or an error string.
pub fn minify(format: Format, input: &str) -> Result<String, String> {
    match format {
        Format::Html => html(input),
        Format::Css => css(input),
        Format::JavaScript => minify_javascript(input, false),
        Format::Jsx => jsx(input),
        Format::Json => json(input),
        Format::Xml => xml_like(input, false),
        Format::Svg => xml_like(input, true),
    }
}

fn ws(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == b'\x0c'
}

fn word_char(c: u8) -> bool {
    // Deliberately conservative for UTF-8 (reference word_char): non-ASCII
    // bytes may belong to an identifier, so adjacent whitespace must not be
    // removed (this scanner does not decode Unicode identifiers).
    c >= 0x80 || c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'-' || c == b'\\'
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// Validate via the native Rust Jsonic, then strip insignificant whitespace
/// (Minify++ `json`).
fn json(input: &str) -> Result<String, String> {
    jsonic::parse(input).map_err(|e| format!("invalid JSON: {e}"))?;
    let mut output = String::with_capacity(input.len());
    let mut quoted = false;
    let mut escaped = false;
    for c in input.bytes() {
        if quoted {
            output.push(c as char);
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                quoted = false;
            }
        } else if c == b'"' {
            quoted = true;
            output.push(c as char);
        } else if !ws(c) {
            output.push(c as char);
        }
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// CSS
// ---------------------------------------------------------------------------

fn css_needs_space(left: u8, right: u8) -> bool {
    if (word_char(left) && word_char(right))
        || (left == b'/' && right == b'*')
        || (left == b'*' && right == b'/')
    {
        return true;
    }
    if (word_char(left) || left == b'*' || left == b'\'' || left == b'"')
        && (right == b'.' || right == b'#' || right == b'[' || right == b'(' || right == b'*')
    {
        return true;
    }
    if (left == b')'
        || left == b']'
        || left == b'%'
        || left == b'*'
        || left == b'\''
        || left == b'"')
        && (word_char(right)
            || right == b'.'
            || right == b'#'
            || right == b'['
            || right == b'('
            || right == b'*'
            || right == b'\''
            || right == b'"')
    {
        return true;
    }
    if (word_char(left) || left == b')' || left == b']' || left == b'%')
        && (right == b'\'' || right == b'"')
    {
        return true;
    }
    false
}

fn emit_pending_css_space(out: &mut String, pending: &mut bool, next: u8) {
    if !*pending {
        return;
    }
    if !out.is_empty()
        && !ws(out.as_bytes()[out.len() - 1])
        && css_needs_space(out.as_bytes()[out.len() - 1], next)
    {
        out.push(' ');
    }
    *pending = false;
}

fn css_colon_precedes_rule_block(input: &[u8], colon: usize) -> bool {
    let mut parens = 0usize;
    let mut brackets = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut quote = 0u8;
    let mut i = colon + 1;
    while i < input.len() {
        let c = input[i];
        if quoted {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == quote {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            quoted = true;
            quote = c;
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < input.len() && input[i + 1] == b'*' {
            match input[i + 2..].windows(2).position(|w| w == b"*/") {
                Some(rel) => {
                    i += 2 + rel + 2;
                    continue;
                }
                None => return false,
            }
        }
        if c == b'(' {
            parens += 1;
        } else if c == b')' && parens > 0 {
            parens -= 1;
        } else if c == b'[' {
            brackets += 1;
        } else if c == b']' && brackets > 0 {
            brackets -= 1;
        }
        if parens > 0 || brackets > 0 {
            i += 1;
            continue;
        }
        if c == b'{' {
            return true;
        }
        if c == b';' || c == b'}' {
            return false;
        }
        i += 1;
    }
    false
}

/// CSS minification (Minify++ `css`).
fn css(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut pending_space = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\'' || c == b'"' {
            emit_pending_css_space(&mut output, &mut pending_space, c);
            output.push(c as char);
            i += 1;
            let quote = c;
            let mut escaped = false;
            let mut closed = false;
            while i < bytes.len() {
                let q = bytes[i];
                i += 1;
                if q >= 0x80 {
                    let rest = &input[i - 1..];
                    let ch = rest.chars().next().unwrap();
                    output.push(ch);
                    i += ch.len_utf8() - 1;
                    continue;
                }
                output.push(q as char);
                if escaped {
                    escaped = false;
                } else if q == b'\\' {
                    escaped = true;
                } else if q == quote {
                    closed = true;
                    break;
                }
            }
            if !closed {
                return Err("unterminated CSS string".to_string());
            }
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let preserve = i + 2 < bytes.len() && bytes[i + 2] == b'!';
            let rel = bytes[i + 2..].windows(2).position(|w| w == b"*/");
            let Some(rel) = rel else {
                return Err("unterminated CSS comment".to_string());
            };
            let end = i + 2 + rel;
            if preserve {
                emit_pending_css_space(&mut output, &mut pending_space, b'/');
                output.push_str(&input[i..end + 2]);
            } else {
                pending_space = true;
            }
            i = end + 2;
            continue;
        }
        if ws(c) {
            pending_space = true;
            i += 1;
            continue;
        }
        if c >= 0x80 {
            emit_pending_css_space(&mut output, &mut pending_space, c);
            pending_space = false;
            let rest = &input[i..];
            let ch = rest.chars().next().unwrap();
            output.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let punctuation = c == b'{' || c == b'}' || c == b':' || c == b';' || c == b',';
        if punctuation {
            let preserve_before_colon = c == b':'
                && pending_space
                && css_colon_precedes_rule_block(bytes, i)
                && !output.is_empty()
                && output.as_bytes()[output.len() - 1] != b' ';
            pending_space = false;
            while output.as_bytes().last() == Some(&b' ') {
                output.pop();
            }
            if preserve_before_colon {
                output.push(' ');
            }
            output.push(c as char);
        } else {
            if pending_space
                && (c == b'+' || c == b'-')
                && !output.is_empty()
                && output.as_bytes()[output.len() - 1] != b' '
                && ((c == b'+' || c == b'-')
                    || output.as_bytes()[output.len() - 1] == b'+'
                    || output.as_bytes()[output.len() - 1] == b'-')
            {
                output.push(' ');
            } else {
                emit_pending_css_space(&mut output, &mut pending_space, c);
            }
            pending_space = false;
            output.push(c as char);
        }
        i += 1;
    }
    while output.as_bytes().last().map(|b| ws(*b)).unwrap_or(false) {
        output.pop();
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// JavaScript (ASI-safe: comments removed, horizontal whitespace collapsed,
// every significant newline preserved) - Minify++ `minify_javascript`.
// ---------------------------------------------------------------------------

fn minify_javascript(input: &str, preserve_jsx_boundaries: bool) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut pending_space = false;
    let mut pending_newline = false;
    let mut can_start_regex = true;
    let mut pending_control_paren = false;
    let mut control_parens: Vec<bool> = Vec::new();
    let mut block_braces: Vec<bool> = Vec::new();
    let mut last_token = String::new();
    let mut pending_class_brace = false;
    let mut pending_class_expression = false;
    let mut pending_function_brace = false;
    let mut pending_function_expression = false;
    let mut pending_async_expression = false;

    fn emit_pending_js(
        output: &mut String,
        pending_space: &mut bool,
        pending_newline: &mut bool,
        preserve_jsx_boundaries: bool,
        next: u8,
    ) {
        if *pending_newline {
            if !output.is_empty() && output.as_bytes().last() != Some(&b'\n') {
                output.push('\n');
            }
        } else if *pending_space && !output.is_empty() {
            let l = output.as_bytes()[output.len() - 1];
            if (word_char(l) && word_char(next))
                || (l == b'+' && next == b'+')
                || (l == b'-' && next == b'-')
                || (l == b'/' && next == b'/')
                || (l == b'/' && next == b'*')
                || (l == b'*' && next == b'/')
                || (preserve_jsx_boundaries
                    && l == b'<'
                    && (next.is_ascii_alphabetic() || next == b'>' || next == b'/'))
                || (l.is_ascii_digit() && next == b'.')
            {
                output.push(' ');
            }
        }
        *pending_space = false;
        *pending_newline = false;
    }

    let is_control_keyword =
        |word: &str| matches!(word, "if" | "while" | "for" | "with" | "switch" | "catch");
    let is_expr_prefix = |word: &str| {
        matches!(
            word,
            "return"
                | "throw"
                | "case"
                | "delete"
                | "void"
                | "typeof"
                | "new"
                | "in"
                | "instanceof"
                | "yield"
                | "await"
                | "else"
                | "do"
        )
    };

    // Reference statement_position: the character BEFORE the identifier that
    // precedes a label colon must be a statement boundary (start of input,
    // `;`, `{`, `}`, or `)`).
    let is_stmt_boundary = |out: &String, identifier_start: usize| {
        if identifier_start == 0 {
            return true;
        }
        let before = out.as_bytes()[identifier_start - 1];
        before == b';' || before == b'{' || before == b'}' || before == b')'
    };

    let mut i = 0usize;
    while i < input.len() {
        let c = bytes[i];
        if ws(c) {
            if c == b'\n' || c == b'\r' {
                pending_newline = true;
            } else {
                pending_space = true;
            }
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            let begin = i;
            i += 1;
            while i < input.len() {
                let u = bytes[i];
                if !u.is_ascii_alphanumeric() && u != b'_' && u != b'$' {
                    break;
                }
                i += 1;
            }
            let word = &input[begin..i];
            emit_pending_js(
                &mut output,
                &mut pending_space,
                &mut pending_newline,
                preserve_jsx_boundaries,
                word.as_bytes()[0],
            );
            output.push_str(word);

            let was_pending_control_paren = pending_control_paren;
            pending_control_paren =
                is_control_keyword(word) || (was_pending_control_paren && word == "await");
            if word == "async" {
                pending_async_expression = !(last_token.is_empty()
                    || last_token == ";"
                    || last_token == "}block"
                    || last_token == "export"
                    || last_token == "default");
            }
            if word == "function" {
                pending_function_brace = true;
                pending_function_expression = if last_token == "async" {
                    pending_async_expression
                } else {
                    !(last_token.is_empty()
                        || last_token == ";"
                        || last_token == "}block"
                        || last_token == "export"
                        || last_token == "default")
                };
            }
            if word == "class" {
                pending_class_brace = true;
                pending_class_expression = !(last_token.is_empty()
                    || last_token == ";"
                    || last_token == "}block"
                    || last_token == "export"
                    || last_token == "default");
            }
            can_start_regex = pending_control_paren || is_expr_prefix(word);
            last_token = word.to_string();
            continue;
        }
        if c.is_ascii_digit() {
            let begin = i;
            while i < input.len() && bytes[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let number = &input[begin..i];
            emit_pending_js(
                &mut output,
                &mut pending_space,
                &mut pending_newline,
                preserve_jsx_boundaries,
                number.as_bytes()[0],
            );
            output.push_str(number);
            pending_control_paren = false;
            can_start_regex = false;
            last_token = "value".to_string();
            continue;
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            emit_pending_js(
                &mut output,
                &mut pending_space,
                &mut pending_newline,
                preserve_jsx_boundaries,
                c,
            );
            output.push(c as char);
            i += 1;
            let quote = c;
            let mut escaped = false;
            let mut closed = false;
            while i < input.len() {
                let q = bytes[i];
                i += 1;
                output.push(q as char);
                if escaped {
                    escaped = false;
                } else if q == b'\\' {
                    escaped = true;
                } else if q == quote {
                    closed = true;
                    break;
                }
            }
            if !closed {
                return Err(if quote == b'`' {
                    "unterminated JavaScript template literal".to_string()
                } else {
                    "unterminated JavaScript string literal".to_string()
                });
            }
            pending_control_paren = false;
            can_start_regex = false;
            last_token = "\"".to_string();
            continue;
        }
        if c == b'{' {
            emit_pending_js(
                &mut output,
                &mut pending_space,
                &mut pending_newline,
                preserve_jsx_boundaries,
                c,
            );
            let mut is_block = false;
            if pending_class_brace
                || pending_function_brace
                || last_token.is_empty()
                || last_token == ";"
                || last_token == "}block"
                || last_token == ")"
                || last_token == ")control"
                || last_token == "else"
                || last_token == "do"
                || last_token == "try"
                || last_token == "catch"
                || last_token == "finally"
                || last_token == "class"
            {
                is_block = true;
            } else if last_token == ":"
                && (block_braces.is_empty() || *block_braces.last().unwrap())
            {
                // Label `label: { ... }` closes like a block; a ternary/object
                // colon value `{...}` is expression-like. Confirm the fragment
                // before ':' is a bare identifier in statement position.
                let end = output
                    .trim_end_matches(|c: char| ws(c as u8))
                    .len()
                    .saturating_sub(1); // position of ':'
                let mut end2 = end;
                while end2 > 0 && ws(output.as_bytes()[end2 - 1]) {
                    end2 -= 1;
                }
                let mut start2 = end2;
                while start2 > 0 {
                    let ch = output.as_bytes()[start2 - 1];
                    if !(ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'$' || ch >= 0x80) {
                        break;
                    }
                    start2 -= 1;
                }
                let identifier = end2 > start2;
                if identifier && is_stmt_boundary(&output, start2) {
                    is_block = true;
                }
            }
            let expression_body = (pending_class_brace && pending_class_expression)
                || (pending_function_brace && pending_function_expression);
            block_braces.push(if expression_body { false } else { is_block });
            output.push('{');
            pending_class_brace = false;
            pending_class_expression = false;
            pending_function_brace = false;
            pending_function_expression = false;
            pending_async_expression = false;
            pending_control_paren = false;
            can_start_regex = true;
            last_token = "{".to_string();
            i += 1;
            continue;
        }
        if c == b'}' {
            emit_pending_js(
                &mut output,
                &mut pending_space,
                &mut pending_newline,
                preserve_jsx_boundaries,
                c,
            );
            output.push('}');
            let was_block = block_braces.pop().unwrap_or(true);
            pending_control_paren = false;
            can_start_regex = was_block;
            last_token = if was_block {
                "}block".to_string()
            } else {
                "}object".to_string()
            };
            i += 1;
            continue;
        }
        if c == b'(' {
            emit_pending_js(
                &mut output,
                &mut pending_space,
                &mut pending_newline,
                preserve_jsx_boundaries,
                c,
            );
            output.push('(');
            control_parens.push(pending_control_paren);
            pending_control_paren = false;
            can_start_regex = true;
            last_token = "(".to_string();
            i += 1;
            continue;
        }
        if c == b')' {
            emit_pending_js(
                &mut output,
                &mut pending_space,
                &mut pending_newline,
                preserve_jsx_boundaries,
                c,
            );
            output.push(')');
            let was_control = control_parens.last() == Some(&true);
            if !control_parens.is_empty() {
                control_parens.pop();
            }
            pending_control_paren = false;
            can_start_regex = was_control;
            last_token = if was_control {
                ")control".to_string()
            } else {
                ")".to_string()
            };
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < input.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < input.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            pending_newline = true;
            continue;
        }
        if c == b'/' && i + 1 < input.len() && bytes[i + 1] == b'*' {
            let preserve = i + 2 < input.len() && bytes[i + 2] == b'!';
            let rel = bytes[i + 2..].windows(2).position(|w| w == b"*/");
            let Some(rel) = rel else {
                return Err("unterminated JavaScript block comment".to_string());
            };
            let close = i + 2 + rel;
            let had_newline =
                input[i + 2..close].contains('\n') || input[i + 2..close].contains('\r');
            if preserve {
                emit_pending_js(
                    &mut output,
                    &mut pending_space,
                    &mut pending_newline,
                    preserve_jsx_boundaries,
                    b'/',
                );
                output.push_str(&input[i..close + 2]);
            } else if had_newline {
                pending_newline = true;
            } else {
                pending_space = true;
            }
            i = close + 2;
            continue;
        }
        if c == b'/'
            && can_start_regex
            && i + 1 < input.len()
            && (output.is_empty() || output.as_bytes().last() != Some(&b'<'))
        {
            emit_pending_js(
                &mut output,
                &mut pending_space,
                &mut pending_newline,
                preserve_jsx_boundaries,
                c,
            );
            output.push('/');
            i += 1;
            let mut escaped = false;
            let mut in_class = false;
            let mut closed = false;
            while i < input.len() {
                let q = bytes[i];
                i += 1;
                output.push(q as char);
                if escaped {
                    escaped = false;
                    continue;
                }
                if q == b'\\' {
                    escaped = true;
                    continue;
                }
                if q == b'[' {
                    in_class = true;
                } else if q == b']' {
                    in_class = false;
                } else if q == b'/' && !in_class {
                    closed = true;
                    break;
                } else if q == b'\n' || q == b'\r' {
                    break;
                }
            }
            if !closed {
                return Err("unterminated JavaScript regular expression".to_string());
            }
            while i < input.len() && bytes[i].is_ascii_alphabetic() {
                output.push(bytes[i] as char);
                i += 1;
            }
            pending_control_paren = false;
            can_start_regex = false;
            last_token = "value".to_string();
            continue;
        }
        // Generic token (ASCII operators/punctuation, or a non-ASCII char).
        if c >= 0x80 {
            // Copy the full UTF-8 character (do not split multibyte sequences).
            let rest = &input[i..];
            let ch = rest.chars().next().unwrap();
            emit_pending_js(
                &mut output,
                &mut pending_space,
                &mut pending_newline,
                preserve_jsx_boundaries,
                c,
            );
            output.push(ch);
            i += ch.len_utf8();
            pending_control_paren = false;
            can_start_regex = false;
            last_token = ch.to_string();
            continue;
        }
        emit_pending_js(
            &mut output,
            &mut pending_space,
            &mut pending_newline,
            preserve_jsx_boundaries,
            c,
        );
        output.push(c as char);
        pending_control_paren = false;
        if word_char(c) || c == b']' || c == b'.' || c == b'\'' || c == b'"' || c == b'`' {
            can_start_regex = false;
        } else if c == b';'
            || c == b','
            || c == b':'
            || c == b'['
            || c == b'='
            || c == b'!'
            || c == b'?'
            || c == b'&'
            || c == b'|'
            || c == b'+'
            || c == b'-'
            || c == b'*'
            || c == b'%'
            || c == b'<'
            || c == b'>'
            || c == b'/'
        {
            can_start_regex = true;
        }
        last_token = (c as char).to_string();
        i += 1;
    }
    while output.as_bytes().last().map(|b| ws(*b)).unwrap_or(false) {
        output.pop();
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// JSX helpers (Minify++ faithful ports)
// ---------------------------------------------------------------------------

fn looks_like_jsx_start(input: &str, i: usize) -> bool {
    let bytes = input.as_bytes();
    if i + 1 >= bytes.len() || bytes[i] != b'<' {
        return false;
    }
    let n = bytes[i + 1];
    n.is_ascii_alphabetic() || bytes[i + 1] == b'>' || bytes[i + 1] == b'/'
}

fn looks_like_jsx_root_start(input: &str, i: usize) -> bool {
    if !looks_like_jsx_start(input, i) {
        return false;
    }
    let bytes = input.as_bytes();
    if bytes[i + 1] == b'/' || bytes[i + 1] == b'>' {
        return true;
    }
    let mut p = i;
    while p > 0 && ws(bytes[p - 1]) {
        p -= 1;
    }
    if p == 0 {
        return true;
    }
    let prev = bytes[p - 1];
    if matches!(
        prev,
        b'=' | b'('
            | b'['
            | b'{'
            | b','
            | b':'
            | b';'
            | b'?'
            | b'!'
            | b'&'
            | b'|'
            | b'+'
            | b'-'
            | b'*'
            | b'%'
            | b'~'
            | b'^'
            | b'>'
    ) {
        return true;
    }
    let end = p;
    while p > 0 {
        let c = bytes[p - 1];
        if !(c.is_ascii_alphanumeric() || bytes[p - 1] == b'_' || bytes[p - 1] == b'$') {
            break;
        }
        p -= 1;
    }
    let word = &input[p..end];
    matches!(word, "return" | "yield" | "await" | "case" | "throw")
}

fn looks_like_tsx_generic_arrow(input: &str, start: usize, limit: usize) -> bool {
    let bytes = input.as_bytes();
    if start >= limit || bytes[start] != b'<' {
        return false;
    }
    let mut angle = 1usize;
    let mut trailing_comma = false;
    let mut saw_extends = false;
    let mut quoted = false;
    let mut escaped = false;
    let mut quote = 0u8;
    let mut i = start + 1;
    while i < limit {
        let c = bytes[i];
        if quoted {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == quote {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            quoted = true;
            quote = c;
            i += 1;
            continue;
        }
        if c == b'<' {
            angle += 1;
            i += 1;
            continue;
        }
        if c == b'>' {
            if i > start && bytes[i - 1] == b'=' {
                i += 1;
                continue;
            }
            angle -= 1;
            if angle == 0 {
                break;
            }
            i += 1;
            continue;
        }
        if angle == 1 && c == b',' {
            trailing_comma = true;
        }
        i += 1;
    }
    if i >= limit || angle != 0 {
        return false;
    }
    let head = &input[start + 1..i];
    if head.contains("extends") {
        saw_extends = true;
    }
    if !trailing_comma && !saw_extends {
        return false;
    }
    let mut p = i + 1;
    while p < limit && ws(bytes[p]) {
        p += 1;
    }
    if p >= limit || bytes[p] != b'(' {
        return false;
    }
    let mut paren = 0usize;
    let mut q = false;
    let mut esc = false;
    let mut qc = 0u8;
    while p < limit {
        let c = bytes[p];
        if q {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == qc {
                q = false;
            }
            p += 1;
            continue;
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            q = true;
            qc = c;
            p += 1;
            continue;
        }
        if c == b'(' {
            paren += 1;
        } else if c == b')' {
            paren -= 1;
            if paren == 0 {
                p += 1;
                break;
            }
        }
        p += 1;
    }
    if paren != 0 {
        return false;
    }
    while p < limit && ws(bytes[p]) {
        p += 1;
    }
    if p < limit && bytes[p] == b':' {
        p += 1;
        let (mut a, mut b, mut c) = (0usize, 0usize, 0usize);
        while p + 1 < limit {
            let x = bytes[p];
            if x == b'<' {
                a += 1;
            } else if x == b'>' && a > 0 {
                a -= 1;
            } else if x == b'[' {
                b += 1;
            } else if x == b']' && b > 0 {
                b -= 1;
            } else if x == b'{' {
                c += 1;
            } else if x == b'}' && c > 0 {
                c -= 1;
            }
            if a == 0 && b == 0 && c == 0 && bytes[p] == b'=' && bytes[p + 1] == b'>' {
                break;
            }
            p += 1;
        }
    }
    while p < limit && ws(bytes[p]) {
        p += 1;
    }
    p + 1 < limit && bytes[p] == b'=' && bytes[p + 1] == b'>'
}

fn find_nested_jsx_end(input: &str, start: usize, limit: usize) -> Result<usize, String> {
    let bytes = input.as_bytes();
    let mut p = start;
    let mut depth = 0usize;
    let mut started = false;
    while p < limit {
        if bytes[p] == b'<' && looks_like_jsx_start(input, p) {
            let closing = p + 1 < limit && bytes[p + 1] == b'/';
            let mut j = p + 1;
            let mut quoted = false;
            let mut escaped = false;
            let mut quote = 0u8;
            let mut tag_angles = 0usize;
            let mut attr_braces = 0usize;
            while j < limit {
                let c = bytes[j];
                if quoted {
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == quote {
                        quoted = false;
                    }
                    j += 1;
                    continue;
                }
                if c == b'\'' || c == b'"' || (attr_braces > 0 && c == b'`') {
                    quoted = true;
                    quote = c;
                    j += 1;
                    continue;
                }
                if c == b'{' {
                    attr_braces += 1;
                    j += 1;
                    continue;
                }
                if c == b'}' && attr_braces > 0 {
                    attr_braces -= 1;
                    j += 1;
                    continue;
                }
                if attr_braces == 0 && c == b'<' {
                    tag_angles += 1;
                    j += 1;
                    continue;
                }
                if c == b'>' && attr_braces == 0 {
                    if tag_angles > 0 && j > p && bytes[j - 1] == b'=' {
                        j += 1;
                        continue;
                    }
                    if tag_angles > 0 {
                        tag_angles -= 1;
                        j += 1;
                        continue;
                    }
                    j += 1;
                    break;
                }
                j += 1;
            }
            if quoted || j > limit || bytes[j - 1] != b'>' {
                return Err("unterminated JSX tag".to_string());
            }
            let self_closing = j >= 2 && bytes[j - 2] == b'/';
            if !started {
                started = true;
                depth = if self_closing { 0 } else { 1 };
            } else if closing {
                depth = depth.saturating_sub(1);
            } else if !self_closing {
                depth += 1;
            }
            p = j;
            if started && depth == 0 {
                return Ok(p);
            }
            continue;
        }
        if bytes[p] == b'{' {
            let end = find_jsx_expression_end(input, p + 1, limit)?;
            p = end;
            continue;
        }
        p += 1;
    }
    Err("unterminated JSX element".to_string())
}

fn find_jsx_expression_end(input: &str, start: usize, limit: usize) -> Result<usize, String> {
    let bytes = input.as_bytes();
    let mut braces = 1usize;
    let mut can_start_regex = true;
    let mut i = start;
    while i < limit {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            let begin = i;
            i += 1;
            while i < limit {
                let u = bytes[i];
                if !u.is_ascii_alphanumeric() && u != b'_' && u != b'$' {
                    break;
                }
                i += 1;
            }
            let word = &input[begin..i];
            can_start_regex = matches!(
                word,
                "return"
                    | "throw"
                    | "case"
                    | "delete"
                    | "void"
                    | "typeof"
                    | "new"
                    | "in"
                    | "instanceof"
                    | "yield"
                    | "await"
                    | "else"
                    | "do"
            );
            continue;
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            let quote = c;
            i += 1;
            let mut escaped = false;
            let mut closed = false;
            while i < limit {
                let q = bytes[i];
                i += 1;
                if escaped {
                    escaped = false;
                } else if q == b'\\' {
                    escaped = true;
                } else if q == quote {
                    closed = true;
                    break;
                }
            }
            if !closed {
                return Err("unterminated string/template in JSX expression".to_string());
            }
            can_start_regex = false;
            continue;
        }
        if c == b'<' && can_start_regex && looks_like_jsx_start(input, i) {
            if looks_like_tsx_generic_arrow(input, i, limit) {
                i += 1;
                can_start_regex = true;
                continue;
            }
            i = find_nested_jsx_end(input, i, limit)?;
            can_start_regex = false;
            continue;
        }
        if c == b'/' && i + 1 < limit && bytes[i + 1] == b'/' {
            i += 2;
            while i < limit && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            can_start_regex = true;
            continue;
        }
        if c == b'/' && i + 1 < limit && bytes[i + 1] == b'*' {
            let rel = bytes[i + 2..limit].windows(2).position(|w| w == b"*/");
            let Some(rel) = rel else {
                return Err("unterminated JavaScript block comment in JSX expression".to_string());
            };
            i += 2 + rel + 2;
            continue;
        }
        if c == b'/' && can_start_regex {
            i += 1;
            let mut escaped = false;
            let mut in_class = false;
            let mut closed = false;
            while i < limit {
                let q = bytes[i];
                i += 1;
                if escaped {
                    escaped = false;
                    continue;
                }
                if q == b'\\' {
                    escaped = true;
                    continue;
                }
                if q == b'[' {
                    in_class = true;
                } else if q == b']' {
                    in_class = false;
                } else if q == b'/' && !in_class {
                    closed = true;
                    break;
                } else if q == b'\n' || q == b'\r' {
                    break;
                }
            }
            if !closed {
                return Err(
                    "unterminated JavaScript regular expression in JSX expression".to_string(),
                );
            }
            while i < limit && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            can_start_regex = false;
            continue;
        }
        if c == b'{' {
            braces += 1;
            can_start_regex = true;
            i += 1;
            continue;
        }
        if c == b'}' {
            braces -= 1;
            if braces == 0 {
                return Ok(i + 1);
            }
            can_start_regex = false;
            i += 1;
            continue;
        }
        if ws(c) {
            i += 1;
            continue;
        }
        if word_char(c) || c == b')' || c == b']' || c == b'.' {
            can_start_regex = false;
        } else if c == b';'
            || c == b','
            || c == b':'
            || c == b'('
            || c == b'['
            || c == b'='
            || c == b'!'
            || c == b'?'
            || c == b'&'
            || c == b'|'
            || c == b'+'
            || c == b'-'
            || c == b'*'
            || c == b'/'
            || c == b'%'
            || c == b'<'
            || c == b'>'
        {
            can_start_regex = true;
        }
        i += 1;
    }
    Err("unterminated JSX expression".to_string())
}

fn jsx(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut js_start = 0usize;

    fn flush_js(
        input: &str,
        output: &mut String,
        js_start: usize,
        end: usize,
    ) -> Result<(), String> {
        if end <= js_start {
            return Ok(());
        }
        let part = minify_javascript(&input[js_start..end], true)?;
        output.push_str(&part);
        Ok(())
    }

    while i < input.len() {
        if bytes[i] == b'/' && i + 1 < input.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < input.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < input.len() && bytes[i + 1] == b'*' {
            let rel = bytes[i + 2..].windows(2).position(|w| w == b"*/");
            let Some(rel) = rel else {
                return Err("unterminated JavaScript block comment".to_string());
            };
            i += 2 + rel + 2;
            continue;
        }
        // Skip JS regex literals before looking for JSX roots.
        if bytes[i] == b'/' && i + 1 < input.len() && bytes[i + 1] != b'/' && bytes[i + 1] != b'*' {
            let mut p = i;
            while p > js_start && ws(bytes[p - 1]) {
                p -= 1;
            }
            let mut regex_here = p == js_start;
            if !regex_here && p > js_start {
                let prev = bytes[p - 1];
                regex_here = matches!(
                    prev,
                    b'=' | b'('
                        | b'['
                        | b'{'
                        | b','
                        | b':'
                        | b';'
                        | b'?'
                        | b'!'
                        | b'&'
                        | b'|'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'%'
                        | b'~'
                        | b'^'
                        | b'>'
                );
                if !regex_here && (prev.is_ascii_alphabetic() || prev == b'_' || prev == b'$') {
                    let end = p;
                    while p > js_start {
                        let u = bytes[p - 1];
                        if !(u.is_ascii_alphanumeric() || u == b'_' || u == b'$') {
                            break;
                        }
                        p -= 1;
                    }
                    let word = &input[p..end];
                    regex_here = matches!(word, "return" | "throw" | "case" | "yield" | "await");
                }
            }
            if regex_here {
                i += 1;
                let mut escaped = false;
                let mut in_class = false;
                while i < input.len() {
                    let c = bytes[i];
                    i += 1;
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    if c == b'\\' {
                        escaped = true;
                        continue;
                    }
                    if c == b'[' {
                        in_class = true;
                    } else if c == b']' {
                        in_class = false;
                    } else if (c == b'/' && !in_class) || c == b'\n' || c == b'\r' {
                        break;
                    }
                }
                while i < input.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                continue;
            }
        }
        if bytes[i] == b'\'' || bytes[i] == b'"' || bytes[i] == b'`' {
            let q = bytes[i];
            i += 1;
            let mut esc = false;
            while i < input.len() {
                let c = bytes[i];
                i += 1;
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == q {
                    break;
                }
            }
            continue;
        }
        if bytes[i] == b'<' && looks_like_tsx_generic_arrow(input, i, input.len()) {
            i += 1;
            continue;
        }
        if !looks_like_jsx_root_start(input, i) {
            i += 1;
            continue;
        }
        flush_js(input, &mut output, js_start, i)?;

        // Copy one JSX region conservatively: markup/text preserved, {...}
        // expressions recursively minified.
        let mut depth = 0usize;
        let mut p = i;
        let mut started = false;
        while p < input.len() {
            if bytes[p] == b'<' && looks_like_jsx_start(input, p) {
                let closing = p + 1 < input.len() && bytes[p + 1] == b'/';
                let fragment_close =
                    p + 2 < input.len() && bytes[p + 1] == b'/' && bytes[p + 2] == b'>';
                let mut j = p + 1;
                let mut quoted = false;
                let mut escaped = false;
                let mut q = 0u8;
                let mut attribute_braces = 0usize;
                let mut tag_angles = 0usize;
                while j < input.len() {
                    let c = bytes[j];
                    if quoted {
                        if escaped {
                            escaped = false;
                        } else if c == b'\\' {
                            escaped = true;
                        } else if c == q {
                            quoted = false;
                        }
                        j += 1;
                        continue;
                    }
                    if c == b'\'' || c == b'"' || (attribute_braces > 0 && c == b'`') {
                        quoted = true;
                        q = c;
                        j += 1;
                        continue;
                    }
                    if c == b'{' {
                        attribute_braces += 1;
                        j += 1;
                        continue;
                    }
                    if c == b'}' && attribute_braces > 0 {
                        attribute_braces -= 1;
                        j += 1;
                        continue;
                    }
                    if attribute_braces == 0 && c == b'<' {
                        tag_angles += 1;
                        j += 1;
                        continue;
                    }
                    if c == b'>' && attribute_braces == 0 {
                        if tag_angles > 0 && j > p && bytes[j - 1] == b'=' {
                            j += 1;
                            continue;
                        }
                        if tag_angles > 0 {
                            tag_angles -= 1;
                            j += 1;
                            continue;
                        }
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                if quoted || j > input.len() || bytes[j - 1] != b'>' {
                    return Err("unterminated JSX tag".to_string());
                }
                let self_closing = j >= 2 && bytes[j - 2] == b'/';
                // Preserve tag spelling; minify attribute-brace expressions.
                {
                    let mut attr_quote = false;
                    let mut attr_escaped = false;
                    let mut attr_q = 0u8;
                    let mut k = p;
                    while k < j {
                        let tc = bytes[k];
                        if attr_quote {
                            output.push(tc as char);
                            if attr_escaped {
                                attr_escaped = false;
                            } else if tc == b'\\' {
                                attr_escaped = true;
                            } else if tc == attr_q {
                                attr_quote = false;
                            }
                            k += 1;
                            continue;
                        }
                        if tc == b'\'' || tc == b'"' {
                            attr_quote = true;
                            attr_q = tc;
                            output.push(tc as char);
                            k += 1;
                            continue;
                        }
                        if tc == b'{' {
                            let end = find_jsx_expression_end(input, k + 1, j)?;
                            let expr = jsx(&input[k + 1..end - 1])?;
                            output.push('{');
                            output.push_str(&expr);
                            output.push('}');
                            k = end;
                            continue;
                        }
                        output.push(tc as char);
                        k += 1;
                    }
                }
                if !started {
                    started = true;
                    depth = if self_closing { 0 } else { 1 };
                } else if closing || fragment_close {
                    depth = depth.saturating_sub(1);
                } else if !self_closing {
                    depth += 1;
                }
                p = j;
                if started && depth == 0 {
                    break;
                }
                continue;
            }
            if bytes[p] == b'{' {
                let end = find_jsx_expression_end(input, p + 1, input.len())?;
                let expr = jsx(&input[p + 1..end - 1])?;
                output.push('{');
                output.push_str(&expr);
                output.push('}');
                p = end;
                continue;
            }
            // Preserve JSX text exactly (including UTF-8 chars).
            let rest = &input[p..];
            let ch = rest.chars().next().unwrap();
            output.push(ch);
            p += ch.len_utf8();
        }
        if !started || depth != 0 {
            return Err("unterminated JSX element".to_string());
        }
        i = p;
        js_start = i;
    }
    flush_js(input, &mut output, js_start, input.len())?;
    Ok(output)
}
pub fn xml(input: &str) -> Result<String, String> {
    xml_like(input, false)
}

pub fn svg(input: &str) -> Result<String, String> {
    xml_like(input, true)
}

fn xml_like(input: &str, _svg_mode: bool) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < input.len() {
        if bytes[i] == b'<' && input[i..].starts_with("<!--") {
            let rel = input[i + 4..].find("-->");
            let Some(rel) = rel else {
                return Err("unterminated XML comment".to_string());
            };
            i += 4 + rel + 3;
            continue;
        }
        if bytes[i] == b'<' && input[i..].starts_with("<![CDATA[") {
            let rel = input[i + 9..].find("]]>");
            let Some(rel) = rel else {
                return Err("unterminated CDATA section".to_string());
            };
            let end = i + 9 + rel + 3;
            output.push_str(&input[i..end]);
            i = end;
            continue;
        }
        if bytes[i] == b'<' && input[i..].starts_with("<?") {
            let rel = input[i + 2..].find("?>");
            let Some(rel) = rel else {
                return Err("unterminated XML processing instruction".to_string());
            };
            let end = i + 2 + rel + 2;
            output.push_str(&input[i..end]);
            i = end;
            continue;
        }
        if bytes[i] == b'<' {
            let mut j = i + 1;
            let mut quoted = false;
            let mut quote = 0u8;
            while j < input.len() {
                let c = bytes[j];
                if quoted {
                    if c == quote {
                        quoted = false;
                    }
                } else if c == b'\'' || c == b'"' {
                    quoted = true;
                    quote = c;
                } else if c == b'>' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            if quoted || j > input.len() || j == 0 || input.as_bytes()[j - 1] != b'>' {
                return Err("unterminated XML tag".to_string());
            }
            let mut ws_pending = false;
            let mut in_quote = false;
            let mut q = 0u8;
            for k in i..j {
                let c = bytes[k];
                if in_quote {
                    output.push(c as char);
                    if c == q {
                        in_quote = false;
                    }
                } else if c == b'\'' || c == b'"' {
                    if ws_pending
                        && !output.is_empty()
                        && output.as_bytes()[output.len() - 1] != b'<'
                        && output.as_bytes()[output.len() - 1] != b' '
                    {
                        output.push(' ');
                    }
                    ws_pending = false;
                    in_quote = true;
                    q = c;
                    output.push(c as char);
                } else if ws(c) {
                    ws_pending = true;
                } else if c >= 0x80 {
                    if ws_pending
                        && !output.is_empty()
                        && output.as_bytes()[output.len() - 1] != b' '
                        && output.as_bytes()[output.len() - 1] != b'<'
                    {
                        output.push(' ');
                    }
                    ws_pending = false;
                    let rest = &input[k..];
                    let ch = rest.chars().next().unwrap();
                    output.push(ch);
                } else {
                    let after_tag_prefix = !output.is_empty()
                        && (output.as_bytes()[output.len() - 1] == b'<'
                            || (output.as_bytes()[output.len() - 1] == b'/'
                                && output.len() >= 2
                                && output.as_bytes()[output.len() - 2] == b'<'));
                    if ws_pending
                        && !output.is_empty()
                        && (after_tag_prefix
                            || (output.as_bytes()[output.len() - 1] != b'/'
                                && c != b'>'
                                && c != b'/'))
                    {
                        output.push(' ');
                    }
                    ws_pending = false;
                    output.push(c as char);
                }
            }
            i = j;
            continue;
        }
        if ws(bytes[i]) {
            let j = i;
            while i < input.len() && ws(bytes[i]) {
                i += 1;
            }
            output.push_str(&input[j..i]);
            continue;
        }
        if bytes[i] >= 0x80 {
            let rest = &input[i..];
            let ch = rest.chars().next().unwrap();
            output.push(ch);
            i += ch.len_utf8();
            continue;
        }
        output.push(bytes[i] as char);
        i += 1;
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// HTML (Minify++ `html`)
// ---------------------------------------------------------------------------

fn html(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut pending_space = false;
    let mut raw_tag = String::new();

    let starts_ci = |input: &str, pos: usize, needle: &str| -> bool {
        input[pos..].len() >= needle.len()
            && input[pos..pos + needle.len()].eq_ignore_ascii_case(needle)
    };

    let mut i = 0usize;
    while i < input.len() {
        if !raw_tag.is_empty() {
            let close = format!("</{raw_tag}");
            let _ = close.as_bytes();
            let mut p = i;
            let mut found = false;
            while p < input.len() {
                if !starts_ci(input, p, &close) {
                    p += 1;
                    continue;
                }
                let after = p + close.len();
                if after < input.len() && (bytes[after] == b'>' || ws(bytes[after])) {
                    found = true;
                    break;
                }
                p += 1;
            }
            if !found {
                output.push_str(&input[i..]);
                break;
            }
            output.push_str(&input[i..p]);
            i = p;
            raw_tag.clear();
            continue;
        }
        if bytes[i] == b'<' && input[i..].starts_with("<!--") {
            let rel = input[i + 4..].find("-->");
            let Some(rel) = rel else {
                return Err("unterminated HTML comment".to_string());
            };
            let end = i + 4 + rel;
            let preserve = starts_ci(input, i, "<!--[if")
                || input[i..].starts_with("<!--#")
                || input[i..].starts_with("<!--!");
            if preserve {
                if pending_space && !output.is_empty() {
                    output.push(' ');
                }
                pending_space = false;
                output.push_str(&input[i..end + 3]);
            }
            i = end + 3;
            continue;
        }
        if bytes[i] == b'<' {
            if pending_space && !output.is_empty() && !ws(output.as_bytes()[output.len() - 1]) {
                output.push(' ');
            }
            pending_space = false;
            let mut j = i + 1;
            let mut quoted = false;
            let mut quote = 0u8;
            while j < input.len() {
                let c = bytes[j];
                if quoted {
                    if c == b'\\' {
                        j += 1;
                    } else if c == quote {
                        quoted = false;
                    }
                } else if c == b'\'' || c == b'"' {
                    quoted = true;
                    quote = c;
                } else if c == b'>' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            if quoted || j > input.len() || input.as_bytes()[j - 1] != b'>' {
                return Err("unterminated HTML tag".to_string());
            }
            let mut tag_space = false;
            let mut in_quote = false;
            let mut tag_quote = 0u8;
            let mut k = i;
            while k < j {
                let c = bytes[k];
                if in_quote {
                    output.push(c as char);
                    if c == b'\\' && k + 1 < j {
                        // Consume the escaped character (reference `++k`).
                        output.push(input.as_bytes()[k + 1] as char);
                        k += 1;
                    } else if c == tag_quote {
                        in_quote = false;
                    }
                } else if c == b'\'' || c == b'"' {
                    if tag_space
                        && !output.is_empty()
                        && output.as_bytes()[output.len() - 1] != b'<'
                        && output.as_bytes()[output.len() - 1] != b' '
                    {
                        output.push(' ');
                    }
                    tag_space = false;
                    in_quote = true;
                    tag_quote = c;
                    output.push(c as char);
                } else if ws(c) {
                    tag_space = true;
                } else {
                    let after_tag_prefix = !output.is_empty()
                        && (output.as_bytes()[output.len() - 1] == b'<'
                            || (output.as_bytes()[output.len() - 1] == b'/'
                                && output.len() >= 2
                                && output.as_bytes()[output.len() - 2] == b'<'));
                    if tag_space
                        && !output.is_empty()
                        && (after_tag_prefix
                            || (output.as_bytes()[output.len() - 1] != b'/'
                                && c != b'>'
                                && c != b'/'))
                    {
                        output.push(' ');
                    }
                    tag_space = false;
                    output.push(c as char);
                }
                k += 1;
            }
            let mut n = i + 1;
            while n < j && ws(bytes[n]) {
                n += 1;
            }
            if n < j && bytes[n] != b'/' && bytes[n] != b'!' && bytes[n] != b'?' {
                let mut e = n;
                while e < j
                    && (bytes[e].is_ascii_alphanumeric() || bytes[e] == b'-' || bytes[e] == b':')
                {
                    e += 1;
                }
                let name = input[n..e].to_ascii_lowercase();
                if name == "pre" || name == "textarea" || name == "script" || name == "style" {
                    raw_tag = name;
                }
            }
            i = j;
            continue;
        }
        if ws(bytes[i]) {
            pending_space = true;
            i += 1;
            continue;
        }
        if pending_space && !output.is_empty() {
            output.push(' ');
        }
        pending_space = false;
        if bytes[i] >= 0x80 {
            let rest = &input[i..];
            let ch = rest.chars().next().unwrap();
            output.push(ch);
            i += ch.len_utf8();
            continue;
        }
        output.push(bytes[i] as char);
        i += 1;
    }
    while output.as_bytes().last().map(|b| ws(*b)).unwrap_or(false) {
        output.pop();
    }
    let trimmed = output.trim_start_matches(|c: char| ws(c as u8));
    Ok(trimmed.to_string())
}

// ---------------------------------------------------------------------------
// JSX (Minify++ `jsx`): JavaScript with JSX boundaries preserved.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_format_strips_whitespace_and_validates() {
        assert_eq!(
            json(" { \"a\" : 1, \"s\" : \"a b\", \"x\" : [ true, null ] } ").unwrap(),
            "{\"a\":1,\"s\":\"a b\",\"x\":[true,null]}"
        );
        assert!(json("{\"a\":}").is_err());
    }

    #[test]
    fn css_format_handles_comments_strings_and_spacing() {
        assert_eq!(
            css("/*x*/ body  { color : red ; margin : 0  10px ; }").unwrap(),
            "body{color:red;margin:0 10px;}"
        );
        let license = css("/*!license*/ .x { content: \"a  b\"; }").unwrap();
        assert!(license.contains("/*!license*/"));
        assert!(license.contains("\"a  b\""));
        let calc = css("a { width: calc( 100% - 1px ) ; }").unwrap();
        assert!(calc.contains("calc(100% - 1px)"));
    }

    #[test]
    fn xml_and_svg_formats() {
        let xml = xml_like("<a>  <b  x=\"1\"  >hi</b>  </a>", false).unwrap();
        assert_eq!(xml, "<a>  <b x=\"1\">hi</b>  </a>");
        assert_eq!(xml_like("<!--c--><a/>", false).unwrap(), "<a/>");
        assert_eq!(
            xml_like("<a><![CDATA[ keep <raw> ]]></a>", false).unwrap(),
            "<a><![CDATA[ keep <raw> ]]></a>"
        );
        assert_eq!(svg("<svg><!--x--></svg>").unwrap(), "<svg></svg>");
    }

    #[test]
    fn javascript_formats() {
        assert_eq!(
            minify_javascript("function f ( a , b ) { return a  +  b ; }", false).unwrap(),
            "function f(a,b){return a+b;}"
        );
        let asi = minify_javascript("var a = 1;\nvar b = a\n+ 1;", false).unwrap();
        assert!(asi.contains('\n'), "ASI-significant newline must survive");
        assert_eq!(
            minify_javascript("// comment\nvar x = 1;", false).unwrap(),
            "var x=1;"
        );
    }

    #[test]
    fn jsx_preserves_boundaries() {
        let out = jsx("const el = <div  className=\"a\"  >hello</div>;").unwrap();
        assert!(out.contains("<div"), "JSX tag structure preserved");
        assert!(out.contains("</div>"));
    }

    #[test]
    fn idempotence_on_representative_inputs() {
        let inputs = [
            (Format::Html, "<div  class=\"a\" >  <p> hi </p>  </div>"),
            (
                Format::Css,
                "body { color : red ; } /* x */ p { margin : 0 }",
            ),
            (Format::Json, "{ \"a\" : 1, \"b\" : [ 1, 2 ] }"),
            (Format::Xml, "<a> <b/> </a>"),
        ];
        for (format, input) in inputs {
            let once = minify(format, input).unwrap();
            let twice = minify(format, &once).unwrap();
            assert_eq!(once, twice, "{format:?} not idempotent");
        }
    }

    #[test]
    fn adversarial_inputs_never_panic() {
        for input in [
            "<",
            "</",
            "<!--",
            "<!--x",
            "<![CDATA[",
            "<?",
            "\"",
            "'",
            "/",
            "/*",
            "//",
            "{",
            "}",
            "(",
            ")",
            "1e",
            "0x",
            "<div",
            "const",
            "return",
            "\u{0}\u{1}",
        ] {
            for format in [
                Format::Html,
                Format::Css,
                Format::JavaScript,
                Format::Jsx,
                Format::Json,
                Format::Xml,
                Format::Svg,
            ] {
                let _ = minify(format, input); // must not panic
            }
        }
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore = "hardware/performance-sensitive; use examples/bench.rs for real numbers"]
    fn per_format_throughput_and_output() {
        let cases: &[(Format, &str)] = &[
            (
                Format::Html,
                "<div  class=\"a\" >  <p> hello world </p>  <span> x </span> </div>",
            ),
            (
                Format::Css,
                "body { color : red ; margin : 0  10px ; } /* c */ p { padding : 1px 2px ; }",
            ),
            (
                Format::Json,
                "{ \"a\" : 1, \"b\" : [ 1, 2, 3 ], \"c\" : { \"d\" : \"e\" } }",
            ),
            (Format::Xml, "<a> <b  x=\"1\" > text </b> <c/> </a>"),
            (
                Format::Svg,
                "<svg> <rect  width=\"10\"  height=\"10\" /> <!-- c --> </svg>",
            ),
            (
                Format::JavaScript,
                "function f ( a , b ) { return a  +  b ; } // comment",
            ),
            (
                Format::Jsx,
                "const el = <div  className=\"a\" > hello </div>;",
            ),
        ];
        const ROUNDS: usize = 50_000;
        for (format, input) in cases {
            let start = Instant::now();
            let mut total = 0usize;
            for _ in 0..ROUNDS {
                total += minify(*format, input).unwrap().len();
            }
            let elapsed = start.elapsed();
            let in_bytes = input.len() as f64 * ROUNDS as f64;
            let mibs = in_bytes / elapsed.as_secs_f64() / (1024.0 * 1024.0);
            println!(
                "{:?}: input {:.1} MiB/s, output {} bytes/doc (out total {}) in {elapsed:?}",
                format,
                mibs,
                total / ROUNDS,
                total
            );
            assert!(mibs > 1.0, "{format:?} suspiciously slow");
        }
    }
}
