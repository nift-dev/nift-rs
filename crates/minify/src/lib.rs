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

use std::collections::VecDeque;

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
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'-'
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
    if (left == b')' || left == b']' || left == b'%' || left == b'*' || left == b'\'' || left == b'"')
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
            if pending_space && (c == b'+' || c == b'-') && !output.is_empty() && output.as_bytes()[output.len() - 1] != b' ' {
                output.push(' ');
            } else if pending_space && !output.is_empty()
                && (output.as_bytes()[output.len() - 1] == b'+' || output.as_bytes()[output.len() - 1] == b'-')
                && output.as_bytes()[output.len() - 1] != b' '
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
    let mut last_token: VecDeque<char> = VecDeque::new();
    let mut pending_class_brace = false;
    let mut pending_class_expression = false;
    let mut pending_function_brace = false;
    let mut pending_function_expression = false;
    let mut pending_async_expression = false;

    let last = |o: &String| o.chars().last().map(|c| c as u8).unwrap_or(0);

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
            let word_l = word_char(l);
            let word_n = word_char(next);
            if (word_l && word_n)
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

    fn copy_quoted_js(
        input: &str,
        i: &mut usize,
        quote: u8,
        output: &mut String,
        pending_space: &mut bool,
        pending_newline: &mut bool,
        preserve_jsx_boundaries: bool,
    ) -> Result<(), String> {
        emit_pending_js(
            output,
            pending_space,
            pending_newline,
            preserve_jsx_boundaries,
            quote,
        );
        output.push(input.as_bytes()[*i] as char);
        *i += 1;
        let mut escaped = false;
        while *i < input.len() {
            let q = input.as_bytes()[*i];
            *i += 1;
            output.push(q as char);
            if escaped {
                escaped = false;
            } else if q == b'\\' {
                escaped = true;
            } else if q == quote {
                return Ok(());
            }
        }
        Err(if quote == b'`' {
            "unterminated JavaScript template literal".to_string()
        } else {
            "unterminated JavaScript string literal".to_string()
        })
    }

    let is_control_keyword = |word: &str| matches!(word, "if" | "while" | "for" | "with" | "switch" | "catch");
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
            emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, word.as_bytes()[0]);
            output.push_str(word);

            let was_pending_control_paren = pending_control_paren;
            pending_control_paren = is_control_keyword(word)
                || (was_pending_control_paren && word == "await");
            if word == "async" {
                pending_async_expression = !(last_token.is_empty()
                    || last_token.back() == Some(&';')
                    || last_token.iter().collect::<String>().ends_with("}block")
                    || last_token.back() == Some(&'t')
                    && (last_token.iter().collect::<String>().ends_with("export")
                        || last_token.iter().collect::<String>().ends_with("default")));
            }
            if word == "function" {
                pending_function_brace = true;
                pending_function_expression = if last_token.iter().collect::<String>().ends_with("async") {
                    pending_async_expression
                } else {
                    !(last_token.is_empty()
                        || last_token.back() == Some(&';')
                        || last_token.iter().collect::<String>().ends_with("}block")
                        || last_token.back() == Some(&'t')
                        && (last_token.iter().collect::<String>().ends_with("export")
                            || last_token.iter().collect::<String>().ends_with("default")))
                };
            }
            if word == "class" {
                pending_class_brace = true;
                pending_class_expression = !(last_token.is_empty()
                    || last_token.back() == Some(&';')
                    || last_token.iter().collect::<String>().ends_with("}block")
                    || last_token.back() == Some(&'t')
                    && (last_token.iter().collect::<String>().ends_with("export")
                        || last_token.iter().collect::<String>().ends_with("default")));
            }
            can_start_regex = pending_control_paren || is_expr_prefix(word);
            last_token = word.chars().collect();
            continue;
        }
        if c.is_ascii_digit() {
            let begin = i;
            while i < input.len() && bytes[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let number = &input[begin..i];
            emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, number.as_bytes()[0]);
            output.push_str(number);
            can_start_regex = false;
            last_token = number.chars().collect();
            continue;
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            copy_quoted_js(input, &mut i, c, &mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries)?;
            can_start_regex = false;
            last_token.clear();
            last_token.push_back('"');
            continue;
        }
        if c == b'{' {
            emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, c);
            output.push(c as char);
            let block = pending_class_brace || pending_function_brace || pending_class_expression || pending_function_expression;
            last_token = if block { "}block".chars().collect() } else { "{".chars().collect() };
            pending_class_brace = false;
            pending_function_brace = false;
            pending_class_expression = false;
            pending_function_expression = false;
            can_start_regex = false;
            i += 1;
            continue;
        }
        if c == b'}' {
            emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, c);
            output.push(c as char);
            can_start_regex = false;
            last_token.clear();
            last_token.push_back('}');
            i += 1;
            continue;
        }
        if c == b'(' {
            emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, c);
            output.push(c as char);
            control_parens.push(pending_control_paren);
            pending_control_paren = false;
            can_start_regex = true;
            last_token.clear();
            last_token.push_back('(');
            i += 1;
            continue;
        }
        if c == b')' {
            emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, c);
            output.push(c as char);
            let was_control = control_parens.last() == Some(&true);
            if !control_parens.is_empty() {
                control_parens.pop();
            }
            pending_control_paren = false;
            can_start_regex = was_control;
            last_token.clear();
            last_token.push_back(if was_control { ')' } else { ')' });
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
            let had_newline = input[i + 2..close].contains('\n') || input[i + 2..close].contains('\r');
            if preserve {
                emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, b'/');
                output.push_str(&input[i..close + 2]);
            } else if had_newline {
                pending_newline = true;
            } else {
                pending_space = true;
            }
            i = close + 2;
            continue;
        }
        if c == b'/' && can_start_regex && i + 1 < input.len() && (output.is_empty() || output.as_bytes().last() != Some(&b'<')) {
            emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, c);
            output.push(input.as_bytes()[i] as char);
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
                return Err("unterminated JavaScript regex literal".to_string());
            }
            can_start_regex = false;
            last_token.clear();
            last_token.push_back('/');
            continue;
        }
        if c == b'/' || c == b':' || c == b'?' || c == b'|' || c == b'&' || c == b'=' || c == b'+' || c == b'-' || c == b'*' || c == b'%' || c == b'^' || c == b'~' || c == b'!' || c == b'<' || c == b'>' || c == b',' || c == b';' || c == b'.' {
            emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, c);
            output.push(c as char);
            can_start_regex = false;
            last_token.clear();
            last_token.push_back(c as char);
            i += 1;
            continue;
        }
        // Non-ASCII: keep byte-for-byte.
        emit_pending_js(&mut output, &mut pending_space, &mut pending_newline, preserve_jsx_boundaries, c);
        output.push(c as char);
        can_start_regex = false;
        last_token.clear();
        last_token.push_back(c as char);
        i += 1;
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// XML / SVG (Minify++ `minify_xml_like`)
// ---------------------------------------------------------------------------

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
                    if ws_pending && !output.is_empty() && output.as_bytes()[output.len() - 1] != b'<' && output.as_bytes()[output.len() - 1] != b' ' {
                        output.push(' ');
                    }
                    ws_pending = false;
                    in_quote = true;
                    q = c;
                    output.push(c as char);
                } else if ws(c) {
                    ws_pending = true;
                } else {
                    let after_tag_prefix = !output.is_empty()
                        && (output.as_bytes()[output.len() - 1] == b'<'
                            || (output.as_bytes()[output.len() - 1] == b'/'
                                && output.len() >= 2
                                && output.as_bytes()[output.len() - 2] == b'<'));
                    if ws_pending && !output.is_empty()
                        && (after_tag_prefix
                            || (output.as_bytes()[output.len() - 1] != b'/' && c != b'>' && c != b'/'))
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
            let close_bytes = close.as_bytes();
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
                i = input.len();
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
            if pending_space && !output.is_empty() && !ws(bytes[output.len() - 1]) {
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
            for k in i..j {
                let c = bytes[k];
                if in_quote {
                    output.push(c as char);
                    if c == b'\\' && k + 1 < j {
                        output.push(input.as_bytes()[k + 1] as char);
                    } else if c == tag_quote {
                        in_quote = false;
                    }
                } else if c == b'\'' || c == b'"' {
                    if tag_space && !output.is_empty() && output.as_bytes()[output.len() - 1] != b'<' && output.as_bytes()[output.len() - 1] != b' ' {
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
                    if tag_space && !output.is_empty()
                        && (after_tag_prefix
                            || (output.as_bytes()[output.len() - 1] != b'/' && c != b'>' && c != b'/'))
                    {
                        output.push(' ');
                    }
                    tag_space = false;
                    output.push(c as char);
                }
            }
            let mut n = i + 1;
            while n < j && ws(bytes[n]) {
                n += 1;
            }
            if n < j && bytes[n] != b'/' && bytes[n] != b'!' && bytes[n] != b'?' {
                let mut e = n;
                while e < j && (bytes[e].is_ascii_alphanumeric() || bytes[e] == b'-' || bytes[e] == b':') {
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
    if matches!(prev, b'=' | b'(' | b'[' | b'{' | b',' | b':' | b';' | b'?' | b'!' | b'&' | b'|' | b'+' | b'-' | b'*' | b'%' | b'~' | b'^' | b'>') {
        return true;
    }
    let mut end = p;
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

fn jsx(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < input.len() {
        if bytes[i] == b'<' && looks_like_jsx_start(input, i) {
            // Copy the JSX tag verbatim (attributes already compacted by
            // upstream; preserve structure exactly as authored between < >).
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
                return Err("unterminated JSX tag".to_string());
            }
            output.push_str(&input[i..j]);
            i = j;
            continue;
        }
        // JavaScript portion: feed the remaining text through the ASI-safe JS
        // minifier, which preserves JSX boundaries.
        let rest = &input[i..];
        let minified = minify_javascript(rest, true)?;
        output.push_str(&minified);
        break;
    }
    Ok(output)
}

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
        assert_eq!(
            xml_like("<!--c--><a/>", false).unwrap(),
            "<a/>"
        );
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
        assert_eq!(minify_javascript("// comment\nvar x = 1;", false).unwrap(), "var x=1;");
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
            (Format::Css, "body { color : red ; } /* x */ p { margin : 0 }"),
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
            "<", "</", "<!--", "<!--x", "<![CDATA[", "<?", "\"", "'", "/", "/*", "//",
            "{", "}", "(", ")", "1e", "0x", "<div", "const", "return", "\u{0}\u{1}",
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
    fn per_format_throughput_and_output() {
        let cases: &[(Format, &str)] = &[
            (Format::Html, "<div  class=\"a\" >  <p> hello world </p>  <span> x </span> </div>"),
            (Format::Css, "body { color : red ; margin : 0  10px ; } /* c */ p { padding : 1px 2px ; }"),
            (Format::Json, "{ \"a\" : 1, \"b\" : [ 1, 2, 3 ], \"c\" : { \"d\" : \"e\" } }"),
            (Format::Xml, "<a> <b  x=\"1\" > text </b> <c/> </a>"),
            (Format::Svg, "<svg> <rect  width=\"10\"  height=\"10\" /> <!-- c --> </svg>"),
            (Format::JavaScript, "function f ( a , b ) { return a  +  b ; } // comment"),
            (Format::Jsx, "const el = <div  className=\"a\" > hello </div>;"),
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
