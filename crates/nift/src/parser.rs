//! Parser kernel I (NR2): literal text, comments, escaping, `$[...]` value
//! lookup, deterministic + time/platform-dependent metadata, `@content` and
//! `@if` (with `@else`/`@else @if` chains), all through the [`RenderHost`]
//! seam.
//!
//! Architectural rules (from the frozen programme):
//! - the parser never touches IO directly; every source read goes through
//!   [`RenderHost::read_source`], every value binding through
//!   [`RenderHost::binding`], every geometry/metadata path through the host;
//! - a render is a **function** of (host, identity, sources), returning
//!   `Result` — there is no long-lived mutable parser object;
//! - syntax/semantics mirror the frozen C++ reference; where the contract does
//!   not answer a question the reference answers by behaviour, that behaviour
//!   is pinned by differential tests and recorded, not silently canonized.
//!
//! Deliberately NOT in NR2 (owning checkpoints later): `@for`/loop/collections
//! and expression functions (NR3), `@input`/`@json`/JSON Schema/`@getenv`/
//! `@dep`/`@ent` and the filesystem host (NR4), `@pathto` geometry (NR5), the
//! public Engine (NR6). Templates using those render their unknown directives
//! literally until the owning checkpoint lands, matching the reference's
//! literal fallback for unknown function calls.

use crate::error::{ErrorKind, RenderError};
use crate::host::{RenderHost, RenderIdentity};
use crate::result::RenderResult;
use crate::source::Source;
use crate::value::Value;
use std::path::{Component, Path, PathBuf};

/// Maximum template parse depth before recursion is rejected.
const MAX_PARSE_DEPTH: usize = 64;

/// Renders a template (optionally composed with a page source) through the
/// host seam. `page` is the `@content` source; when provided, exactly one
/// `@content` is required in the rendered output.
pub fn render(
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    template: &Source,
    page: Option<&Source>,
) -> Result<RenderResult, RenderError> {
    let (template_text, template_identity) = resolve_source(host, template)?;
    let mut state = RenderState::default();
    state
        .input_stack
        .push(template_identity.to_string_lossy().to_string());
    let output = parse(
        &mut state,
        host,
        identity,
        page,
        &template_text,
        &template_identity,
        0,
    )?;
    state.input_stack.pop();
    if page.is_some() && state.content_count != 1 {
        return Err(RenderError::new(
            ErrorKind::Render,
            "templated tracked items must execute exactly one @content; add @content through the template/input graph or omit the tracked template field",
        ));
    }
    Ok(RenderResult::new(output))
}

/// Per-render mutable state that persists across recursive parses.
#[derive(Default)]
struct RenderState {
    content_count: usize,
    html_comment_depth: usize,
    code_block_depth: usize,
    input_stack: Vec<String>,
}

fn parse(
    state: &mut RenderState,
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    page: Option<&Source>,
    text: &str,
    source_path: &Path,
    depth: usize,
) -> Result<String, RenderError> {
    if depth > MAX_PARSE_DEPTH {
        return Err(error_at(
            ErrorKind::Parse,
            "maximum template parse depth exceeded (possible recursion)",
            text,
            0,
            source_path,
        ));
    }

    let base_code_block_depth = state.code_block_depth;
    let mut output = String::with_capacity(text.len() + 64);
    let len = text.len();
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < len {
        // Escaping: \@ \$ \# emit the next character literally.
        if i + 1 < len && bytes[i] == b'\\' && matches!(bytes[i + 1], b'@' | b'$' | b'#') {
            output.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }

        // Block comment <#-- ... --#>.
        if text[i..].starts_with("<#--") {
            match text[i + 4..].find("--#>") {
                Some(offset) => {
                    i += 4 + offset + 4;
                    continue;
                }
                None => {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "open comment '<#--' has no close '--#>'",
                        text,
                        i,
                        source_path,
                    ));
                }
            }
        }
        // Line comments @# and @//.
        if text[i..].starts_with("@#") || text[i..].starts_with("@//") {
            match text[i..].find('\n') {
                Some(offset) => i += offset,
                None => i = len,
            }
            continue;
        }

        // <pre*>/<code> literal '<' escaping, matching stripped Nift.
        if text[i..].starts_with("<!--") {
            state.html_comment_depth += 1;
        }
        if text[i..].starts_with("-->") && state.html_comment_depth > 0 {
            state.html_comment_depth -= 1;
        }
        if bytes[i] == b'<' && state.html_comment_depth == 0 {
            let closes_pre =
                text[i + 1..].starts_with("/pre") && i + 5 < len && bytes[i + 5] == b'>';
            let opens_pre = text[i + 1..].starts_with("pre")
                && i + 4 < len
                && matches!(bytes[i + 4], b'>' | b' ' | b'\t' | b'\r' | b'\n');
            if closes_pre {
                if state.code_block_depth > base_code_block_depth {
                    state.code_block_depth -= 1;
                } else {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "</pre> close tag has no preceding <pre*> open tag",
                        text,
                        i,
                        source_path,
                    ));
                }
            }
            let code_tag = text[i + 1..].starts_with("code") || text[i + 1..].starts_with("/code");
            if state.code_block_depth > 0 && !code_tag {
                output += "&lt;";
            } else {
                output.push('<');
            }
            if opens_pre {
                state.code_block_depth += 1;
            }
            i += 1;
            continue;
        }

        // $[...] value lookup.
        if text[i..].starts_with("$[") {
            if let Some(end) = scan_brackets(text, i + 2) {
                let key = &text[i + 2..end];
                match resolve_value(host, identity, key) {
                    Ok(Resolved::Rendered(value)) => {
                        output += &value;
                        i = end + 1;
                        continue;
                    }
                    Ok(Resolved::ArrayObject(kind)) => {
                        let kind = if kind { "array" } else { "object" };
                        return Err(error_at(
                            ErrorKind::Render,
                            format!("cannot render JSON {kind} $[{key}]; select an element first"),
                            text,
                            i,
                            source_path,
                        ));
                    }
                    Ok(Resolved::Unknown) => {
                        // Unresolvable values fall through to literal emission,
                        // matching the reference.
                    }
                    Err(error) => {
                        let mut error = error;
                        if error.source.is_none() {
                            let (line, column) = line_column(text, i);
                            error.source = Some(source_path.to_string_lossy().to_string());
                            error.line = Some(line);
                            error.column = Some(column);
                        }
                        return Err(error);
                    }
                }
            }
        }

        // @if(...){...} with @else / @else @if chains.
        if text[i..].starts_with("@if(") {
            let condition_close = find_balanced(text, i + 3, b'(', b')').ok_or_else(|| {
                error_at(
                    ErrorKind::Parse,
                    "@if has no matching ')' for its condition",
                    text,
                    i,
                    source_path,
                )
            })?;
            let mut block_open = condition_close + 1;
            while block_open < len && matches!(bytes[block_open], b' ' | b'\t' | b'\r' | b'\n') {
                block_open += 1;
            }
            if block_open >= len || bytes[block_open] != b'{' {
                return Err(error_at(
                    ErrorKind::Parse,
                    "@if(...) must be followed by a '{...}' block",
                    text,
                    i,
                    source_path,
                ));
            }
            let block_close = find_balanced(text, block_open, b'{', b'}').ok_or_else(|| {
                error_at(
                    ErrorKind::Parse,
                    "@if block has no matching '}'",
                    text,
                    block_open,
                    source_path,
                )
            })?;

            let condition_value = evaluate_condition(host, identity, &text[i + 4..condition_close])
                .map_err(|e| {
                    if e.source.is_none() {
                        error_at(ErrorKind::Parse, e.message, text, i, source_path)
                    } else {
                        e
                    }
                })?;
            let control_indent = insertion_indent(&output);
            let insertion_code_block_depth = state.code_block_depth;
            let mut selected = false;
            let mut chain_end = block_close + 1;
            if condition_value {
                let body = normalize_control_block_body(&text[block_open + 1..block_close]);
                let nested = parse(state, host, identity, page, &body, source_path, depth + 1)?;
                append_indented(
                    &mut output,
                    &nested,
                    &control_indent,
                    insertion_code_block_depth,
                );
                selected = true;
            }

            let mut cursor = block_close + 1;
            while cursor < len {
                let whitespace_start = cursor;
                while cursor < len && matches!(bytes[cursor], b' ' | b'\t' | b'\r' | b'\n') {
                    cursor += 1;
                }
                if !text[cursor..].starts_with("else")
                    || (cursor + 4 < len
                        && (bytes[cursor + 4].is_ascii_alphanumeric() || bytes[cursor + 4] == b'_'))
                {
                    chain_end = whitespace_start;
                    break;
                }
                cursor += 4;
                while cursor < len && matches!(bytes[cursor], b' ' | b'\t' | b'\r' | b'\n') {
                    cursor += 1;
                }

                let mut branch_condition = true;
                let mut is_else_if = false;
                if text[cursor..].starts_with("if")
                    && cursor + 2 < len
                    && matches!(bytes[cursor + 2], b'(' | b' ' | b'\t' | b'\r' | b'\n')
                {
                    is_else_if = true;
                    cursor += 2;
                    while cursor < len && matches!(bytes[cursor], b' ' | b'\t' | b'\r' | b'\n') {
                        cursor += 1;
                    }
                    if cursor >= len || bytes[cursor] != b'(' {
                        return Err(error_at(
                            ErrorKind::Parse,
                            "else if must contain a parenthesised condition",
                            text,
                            cursor,
                            source_path,
                        ));
                    }
                    let else_condition_close =
                        find_balanced(text, cursor, b'(', b')').ok_or_else(|| {
                            error_at(
                                ErrorKind::Parse,
                                "else if has no matching ')' for its condition",
                                text,
                                cursor,
                                source_path,
                            )
                        })?;
                    if !selected {
                        branch_condition = evaluate_condition(
                            host,
                            identity,
                            &text[cursor + 1..else_condition_close],
                        )
                        .map_err(|e| {
                            if e.source.is_none() {
                                error_at(ErrorKind::Parse, e.message, text, cursor, source_path)
                            } else {
                                e
                            }
                        })?;
                    }
                    cursor = else_condition_close + 1;
                }

                while cursor < len && matches!(bytes[cursor], b' ' | b'\t' | b'\r' | b'\n') {
                    cursor += 1;
                }
                if cursor >= len || bytes[cursor] != b'{' {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "else/else if must be followed by a '{...}' block",
                        text,
                        cursor,
                        source_path,
                    ));
                }
                let else_block_close =
                    find_balanced(text, cursor, b'{', b'}').ok_or_else(|| {
                        error_at(
                            ErrorKind::Parse,
                            "else/else if block has no matching '}'",
                            text,
                            cursor,
                            source_path,
                        )
                    })?;

                if !selected && branch_condition {
                    let body = normalize_control_block_body(&text[cursor + 1..else_block_close]);
                    let nested = parse(state, host, identity, page, &body, source_path, depth + 1)?;
                    append_indented(
                        &mut output,
                        &nested,
                        &control_indent,
                        insertion_code_block_depth,
                    );
                    selected = true;
                }

                cursor = else_block_close + 1;
                chain_end = cursor;

                if !is_else_if {
                    let mut after_else = cursor;
                    while after_else < len
                        && matches!(bytes[after_else], b' ' | b'\t' | b'\r' | b'\n')
                    {
                        after_else += 1;
                    }
                    if text[after_else..].starts_with("else") {
                        return Err(error_at(
                            ErrorKind::Parse,
                            "plain else must be the final branch of an @if chain",
                            text,
                            after_else,
                            source_path,
                        ));
                    }
                    chain_end = cursor;
                    break;
                }
            }
            i = chain_end;
            continue;
        }

        // General @function dispatch (lowercase names only). NR2 supports
        // "content"; unknown or not-yet-implemented functions fall through to
        // literal emission, matching the reference.
        if bytes[i] == b'@' && i + 1 < len && bytes[i + 1].is_ascii_lowercase() {
            let mut name_end = i + 1;
            while name_end < len && bytes[name_end].is_ascii_lowercase() {
                name_end += 1;
            }
            let function = &text[i + 1..name_end];
            let mut has_parameters = false;
            let mut end = name_end;
            if name_end < len && bytes[name_end] == b'(' {
                let close = find_balanced(text, name_end, b'(', b')').ok_or_else(|| {
                    error_at(
                        ErrorKind::Parse,
                        format!("{function}: malformed parameters"),
                        text,
                        i,
                        source_path,
                    )
                })?;
                has_parameters = true;
                end = close + 1;
            }

            // Parameterised functions are only calls when followed by (...);
            // bare non-content @words stay literal (keeps prose like
            // "Partials & @input" literal).
            if !has_parameters && function != "content" {
                if name_end < len && bytes[name_end] == b'[' {
                    return Err(error_at(
                        ErrorKind::Parse,
                        format!("{function}: expected parentheses for parameters"),
                        text,
                        i,
                        source_path,
                    ));
                }
                output += &text[i..name_end];
                i = name_end;
                continue;
            }

            if function == "content" {
                if has_parameters {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "content: expected 0 parameters",
                        text,
                        i,
                        source_path,
                    ));
                }
                let Some(page) = page else {
                    return Err(error_at(
                        ErrorKind::Render,
                        "@content requires a page source; render with a page and template, or use @input for a partial",
                        text,
                        i,
                        source_path,
                    ));
                };
                state.content_count += 1;
                if state.content_count > 1 {
                    return Err(error_at(
                        ErrorKind::Render,
                        "@content may be executed exactly once for a templated tracked item",
                        text,
                        i,
                        source_path,
                    ));
                }
                let (content_text, content_identity) = resolve_source(host, page)?;
                // The reference only guards against input loops when the
                // content identity is non-empty (text sources without a
                // logical name have an empty identity and are never looped).
                let content_identity_string = content_identity.to_string_lossy().to_string();
                let identity_tracked = !content_identity_string.is_empty();
                if identity_tracked {
                    if state.input_stack.contains(&content_identity_string) {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!("@content would result in an input loop through {content_identity_string}"),
                            text,
                            i,
                            source_path,
                        ));
                    }
                    state.input_stack.push(content_identity_string);
                }
                let nested = parse(
                    state,
                    host,
                    identity,
                    Some(page),
                    &content_text,
                    &content_identity,
                    depth + 1,
                )?;
                if identity_tracked {
                    state.input_stack.pop();
                }
                output += &nested;
                i = end;
                continue;
            }

            // Unknown function (or a not-yet-implemented one): literal.
        }

        output.push(bytes[i] as char);
        i += 1;
    }

    Ok(output)
}

enum Resolved {
    /// Rendered scalar text.
    Rendered(String),
    /// Resolved to an array (true) or object (false) rendered directly.
    ArrayObject(bool),
    /// Nothing resolved; emit the `$[...]` literally.
    Unknown,
}

enum Lookup<'h> {
    Found(&'h Value),
    Unknown,
}

fn resolve_value(
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    raw_key: &str,
) -> Result<Resolved, RenderError> {
    let key = raw_key.trim();
    if key.is_empty() {
        return Ok(Resolved::Unknown);
    }
    match lookup(host, key)? {
        Lookup::Found(value) => Ok(match value {
            Value::Array(_) => Resolved::ArrayObject(true),
            Value::Object(_) => Resolved::ArrayObject(false),
            Value::String(s) => Resolved::Rendered(s.clone()),
            Value::Bool(b) => Resolved::Rendered(if *b { "true" } else { "false" }.to_string()),
            Value::Number(n) => Resolved::Rendered(format_number(*n)),
            Value::Null => Resolved::Rendered("null".to_string()),
        }),
        Lookup::Unknown => {
            // Not a host binding: built-in metadata?
            if built_in_metadata_name(key) {
                if let Some(value) = metadata(host, identity, key) {
                    return Ok(Resolved::Rendered(value));
                }
            }
            Ok(Resolved::Unknown)
        }
    }
}

/// Plain value lookup: `root`, `root.member`, `root[0].member`, ...
/// A missing binding, missing member or out-of-range element is `Unknown`
/// (the reference treats it as an unresolved value that renders literally);
/// accessing a member/element on the wrong value type is an error.
fn lookup<'h>(host: &'h dyn RenderHost, key: &str) -> Result<Lookup<'h>, RenderError> {
    let bytes = key.as_bytes();
    let mut pos = 0;
    let first = bytes[pos];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return Ok(Lookup::Unknown);
    }
    pos += 1;
    while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
        pos += 1;
    }
    let root = &key[..pos];

    let Some(mut current) = host.binding(root) else {
        return Ok(Lookup::Unknown);
    };

    while pos < bytes.len() {
        let c = bytes[pos];
        if c == b'.' {
            pos += 1;
            let start = pos;
            while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos += 1;
            }
            let member = &key[start..pos];
            match current {
                Value::Object(map) => match map.get(member) {
                    Some(next) => current = next,
                    None => return Ok(Lookup::Unknown),
                },
                _ => {
                    return Err(RenderError::new(
                        ErrorKind::Render,
                        format!(
                            "cannot access member '{member}' because the current JSON value is not an object"
                        ),
                    ));
                }
            }
        } else if c == b'[' {
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
            if pos == start || pos >= bytes.len() || bytes[pos] != b']' {
                return Ok(Lookup::Unknown);
            }
            let index = key[start..pos].parse::<usize>().unwrap_or(usize::MAX);
            pos += 1;
            match current {
                Value::Array(array) => match array.get(index) {
                    Some(next) => current = next,
                    None => return Ok(Lookup::Unknown),
                },
                _ => {
                    return Err(RenderError::new(
                        ErrorKind::Render,
                        format!(
                            "cannot access element {index} because the current JSON value is not an array"
                        ),
                    ));
                }
            }
        } else {
            return Ok(Lookup::Unknown);
        }
    }

    Ok(Lookup::Found(current))
}

/// Scalar rendering: strings verbatim; numbers/bools/null as compact JSON
/// (matching the reference). Number formatting reproduces the reference's
/// `std::to_chars` rules exactly, verified against a differential battery
/// captured from the frozen C++ reference (tests/number_formatting.rs):
/// integer-valued doubles within i64 range render as integers; everything else
/// renders as `to_chars(general, 15)` (fixed notation when the decimal
/// exponent is in [-4, 15), otherwise scientific with a signed two-digit
/// exponent, trailing zeros stripped).
fn format_number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n >= -(2f64.powi(63)) && n < 2f64.powi(63) {
        return format!("{}", n as i64);
    }
    if !n.is_finite() {
        // inf/nan cannot arise from JSON; not part of the reference corpus.
        return format!("{}", n);
    }
    format_general15(n)
}

/// `std::to_chars(value, general, 15)`-equivalent formatting.
fn format_general15(n: f64) -> String {
    // 15 significant digits in scientific form (1 digit before the point,
    // 14 after), with round-half-even rounding matching the reference.
    let sci = format!("{:.14e}", n);
    let (mantissa, exponent) = sci.split_once('e').expect("e-notation always contains 'e'");
    let exp: i32 = exponent.parse().expect("valid decimal exponent");
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    if (-4..15).contains(&exp) {
        fixed_notation(mantissa, exp)
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{}{:02}", mantissa, sign, exp.abs())
    }
}

/// Convert a mantissa (in [1,10), possibly signed) and decimal exponent to
/// fixed notation with trailing zeros stripped.
fn fixed_notation(mantissa: &str, exp: i32) -> String {
    let (sign, digits) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", mantissa),
    };
    let digits = digits.replace('.', "");
    let point_pos = 1 + exp; // number of digits before the decimal point
    if point_pos <= 0 {
        format!("{}0.{}{}", sign, "0".repeat((-point_pos) as usize), digits)
    } else if point_pos as usize >= digits.len() {
        format!(
            "{}{}{}",
            sign,
            digits,
            "0".repeat(point_pos as usize - digits.len())
        )
    } else {
        let int_part = &digits[..point_pos as usize];
        let frac_part = digits[point_pos as usize..].trim_end_matches('0');
        if frac_part.is_empty() {
            format!("{}{}", sign, int_part)
        } else {
            format!("{}{}.{}", sign, int_part, frac_part)
        }
    }
}

fn built_in_metadata_name(key: &str) -> bool {
    matches!(
        key,
        "title"
            | "name"
            | "content-path"
            | "output-path"
            | "template-path"
            | "build-time"
            | "build-date"
            | "build-UTC-time"
            | "build-UTC-date"
            | "build-YYYY"
            | "build-YY"
            | "build-timezone"
            | "build-OS"
    )
}

fn metadata(host: &dyn RenderHost, identity: &RenderIdentity, key: &str) -> Option<String> {
    match key {
        "title" => Some(identity.title.clone().unwrap_or_default()),
        "name" => Some(identity.name.clone().unwrap_or_default()),
        "content-path" => Some(host.relative(&host.content_path(identity))),
        "output-path" => Some(host.relative(&host.output_path(identity))),
        "template-path" => Some(identity.template_path.clone().unwrap_or_default()),
        "build-time" => Some(chrono::Local::now().format("%H:%M:%S").to_string()),
        "build-date" => Some(chrono::Local::now().format("%Y-%m-%d").to_string()),
        "build-UTC-time" => Some(chrono::Utc::now().format("%H:%M:%S").to_string()),
        "build-UTC-date" => Some(chrono::Utc::now().format("%Y-%m-%d").to_string()),
        "build-YYYY" => Some(chrono::Local::now().format("%Y").to_string()),
        "build-YY" => Some(chrono::Local::now().format("%y").to_string()),
        "build-timezone" => Some(chrono::Local::now().format("%Z").to_string()),
        "build-OS" => Some(
            if cfg!(windows) {
                "Windows"
            } else if cfg!(target_os = "macos") {
                "macOS"
            } else {
                "Linux"
            }
            .to_string(),
        ),
        _ => None,
    }
}

/// NR2 condition evaluator: scalar literals, plain value lookups and built-in
/// metadata, with reference truthiness (bool, null, number!=0, non-empty
/// string/array/object). The semantic resolution layers mirror `$[...]`:
/// host binding > built-in metadata (so a Context/Engine title binding still
/// beats the built-in title metadata). Comparisons, negation and expression
/// functions are NR3.
fn evaluate_condition(
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    expression: &str,
) -> Result<bool, RenderError> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err(RenderError::new(
            ErrorKind::Parse,
            "@if condition cannot be empty",
        ));
    }
    if let Some(value) = scalar_literal(expression) {
        return Ok(truthy(&value));
    }
    match lookup(host, expression)? {
        Lookup::Found(value) => Ok(truthy(value)),
        Lookup::Unknown => {
            if built_in_metadata_name(expression) {
                if let Some(value) = metadata(host, identity, expression) {
                    return Ok(!value.is_empty());
                }
            }
            Err(RenderError::new(
                ErrorKind::Render,
                format!("unknown value or malformed expression: {expression}"),
            ))
        }
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => *n != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn scalar_literal(text: &str) -> Option<Value> {
    match text {
        "true" => return Some(Value::boolean(true)),
        "false" => return Some(Value::boolean(false)),
        "null" => return Some(Value::null()),
        _ => {}
    }
    let quoted = text.len() >= 2
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\'')));
    if quoted {
        return Some(Value::string(&text[1..text.len() - 1]));
    }
    text.parse::<f64>().ok().map(Value::number)
}

fn resolve_source(
    host: &dyn RenderHost,
    source: &Source,
) -> Result<(String, PathBuf), RenderError> {
    match source {
        Source::Text { text, logical_name } => Ok((
            text.clone(),
            PathBuf::from(logical_name.clone().unwrap_or_default()),
        )),
        Source::Path(path) => {
            let resolved = if path.is_relative() {
                host.root().join(path)
            } else {
                path.clone()
            };
            let identity = lexically_normal(&std::path::absolute(&resolved).unwrap_or(resolved));
            let text = host.read_source(&identity)?.into_owned();
            Ok((text, identity))
        }
    }
}

/// Lexical path normalisation (resolve `.`/`..` without touching the
/// filesystem), matching the reference's `lexically_normal` for the absolute
/// paths used as render identities.
fn lexically_normal(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn find_balanced(text: &str, open: usize, open_char: u8, close_char: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    if open >= text.len() || bytes[open] != open_char {
        return None;
    }
    let mut depth = 0usize;
    let mut quoted = false;
    let mut quote = 0u8;
    let mut i = open;
    while i < text.len() {
        let c = bytes[i];
        if quoted {
            if c == b'\\' && i + 1 < text.len() {
                i += 1;
            } else if c == quote {
                quoted = false;
            }
        } else {
            if c == b'\'' || c == b'"' {
                quoted = true;
                quote = c;
            } else if c == open_char {
                depth += 1;
            } else if c == close_char {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

/// Scans from `start` to the matching `]`, honouring nested brackets and
/// quoted strings.
fn scan_brackets(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut nested = 0usize;
    let mut quoted = false;
    let mut quote = 0u8;
    let mut i = start;
    while i < text.len() {
        let c = bytes[i];
        if quoted {
            if c == b'\\' && i + 1 < text.len() {
                i += 1;
            } else if c == quote {
                quoted = false;
            }
        } else {
            if c == b'\'' || c == b'"' {
                quoted = true;
                quote = c;
            } else if c == b'[' {
                nested += 1;
            } else if c == b']' {
                if nested == 0 {
                    return Some(i);
                }
                nested -= 1;
            }
        }
        i += 1;
    }
    None
}

fn insertion_indent(output: &str) -> String {
    let line = match output.rfind('\n') {
        Some(previous) => &output[previous + 1..],
        None => output,
    };
    if line.chars().all(|c| c == ' ' || c == '\t') {
        line.to_string()
    } else {
        " ".repeat(line.len())
    }
}

fn append_indented(output: &mut String, text: &str, indent: &str, initial_code_block_depth: usize) {
    let mut size = text.len();
    if size > 0 && text.as_bytes()[size - 1] == b'\n' {
        size -= 1;
        if size > 0 && text.as_bytes()[size - 1] == b'\r' {
            size -= 1;
        }
    }
    if size == 0 {
        return;
    }
    let mut code_block_depth = initial_code_block_depth;
    let mut html_comment_depth = 0usize;
    let mut segment_start = 0;
    let mut i = 0;
    while i < size {
        if text[i..].starts_with("<!--") {
            html_comment_depth += 1;
        } else if text[i..].starts_with("-->") && html_comment_depth > 0 {
            html_comment_depth -= 1;
        }
        if html_comment_depth == 0 {
            if text[i..].starts_with("</pre>") && code_block_depth > 0 {
                code_block_depth -= 1;
            } else if is_pre_open(text, i, size) {
                code_block_depth += 1;
            }
        }
        if text.as_bytes()[i] == b'\n' {
            output.push_str(&text[segment_start..=i]);
            if i + 1 < size && code_block_depth == 0 {
                output.push_str(indent);
            }
            segment_start = i + 1;
        }
        i += 1;
    }
    if segment_start < size {
        output.push_str(&text[segment_start..size]);
    }
}

fn is_pre_open(text: &str, pos: usize, size: usize) -> bool {
    if pos + 4 > size || !text[pos..].starts_with("<pre") {
        return false;
    }
    if pos + 4 == size {
        return false;
    }
    matches!(
        text.as_bytes()[pos + 4],
        b'>' | b' ' | b'\t' | b'\r' | b'\n'
    )
}

/// Strip the structural first line of a control block body and remove the
/// common indentation of the remaining non-empty lines (readability only, not
/// output indentation).
fn normalize_control_block_body(body: &str) -> String {
    let mut body = body.to_string();
    let bytes_len = |s: &str| s.len();
    let mut first = 0;
    while first < body.len() && matches!(body.as_bytes()[first], b' ' | b'\t') {
        first += 1;
    }
    if first < body.len() && matches!(body.as_bytes()[first], b'\n' | b'\r') {
        if body.as_bytes()[first] == b'\r'
            && first + 1 < body.len()
            && body.as_bytes()[first + 1] == b'\n'
        {
            first += 1;
        }
        body.drain(..=first);
        while !body.is_empty() && matches!(body.as_bytes()[body.len() - 1], b' ' | b'\t') {
            body.pop();
        }
        if !body.is_empty() && body.as_bytes()[body.len() - 1] == b'\n' {
            body.pop();
            if !body.is_empty() && body.as_bytes()[body.len() - 1] == b'\r' {
                body.pop();
            }
        }

        let mut common = usize::MAX;
        let mut line_start = 0;
        loop {
            let line_end = body[line_start..]
                .find('\n')
                .map(|o| line_start + o)
                .unwrap_or(body.len());
            let end = line_end;
            let mut pos = line_start;
            while pos < end && matches!(body.as_bytes()[pos], b' ' | b'\t' | b'\r') {
                pos += 1;
            }
            if pos < end {
                common = common.min(pos - line_start);
            }
            if line_end == body.len() {
                break;
            }
            line_start = line_end + 1;
        }

        if common != usize::MAX && common > 0 {
            let mut dedented = String::with_capacity(body.len());
            line_start = 0;
            loop {
                let line_end = body[line_start..]
                    .find('\n')
                    .map(|o| line_start + o)
                    .unwrap_or(body.len());
                let end = line_end;
                let mut remove = 0;
                while remove < common
                    && line_start + remove < end
                    && matches!(body.as_bytes()[line_start + remove], b' ' | b'\t')
                {
                    remove += 1;
                }
                dedented.push_str(&body[line_start + remove..end]);
                if line_end == body.len() {
                    break;
                }
                dedented.push('\n');
                line_start = line_end + 1;
            }
            body = dedented;
        }
    }
    let _ = bytes_len; // (kept for symmetry with the reference; no-op)
    body
}

fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;
    for (index, byte) in text.bytes().enumerate() {
        if index >= offset {
            break;
        }
        if byte == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn error_at(
    kind: ErrorKind,
    message: impl Into<String>,
    text: &str,
    offset: usize,
    path: &Path,
) -> RenderError {
    let (line, column) = line_column(text, offset);
    RenderError::new(kind, message).at(path.to_string_lossy(), line, column)
}
