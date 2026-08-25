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
///
/// NR11 hardening: the parser's frame is large (a single ~2000-line directive
/// function), and the guard must fit the smallest common thread stack (1 MiB,
/// the Windows default). Probed on a 1 MiB stack: `@if` nesting overflows
/// around depth 22, so 12 leaves comfortable margin for the outer render
/// frames and other recursion paths. The reference's 64 is not a semantic
/// contract; deep nesting is a safety limit, and no canonical template nests
/// more than a handful of levels.
const MAX_PARSE_DEPTH: usize = 12;

/// The marker emitted by `@paginate`, replaced by the rendered pagination
/// template when the primary page is assembled (reference `\x1dNIFT_PAGINATE\x1d`).
pub(crate) const PAGINATION_MARKER: &str = "\u{1d}NIFT_PAGINATE\u{1d}";

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
    // Reference: a path-backed template registers its root-relative identity as
    // a dependency.
    if let Source::Path(_) = template {
        state.dependencies.insert(host.relative(&template_identity));
    }
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
    let mut result = RenderResult::new(output);
    result.dependencies = state.dependencies;
    result.requirements = state.requirements;
    Ok(result)
}

/// Renders a tracked page through a project-aware host (NR8): content/template
/// geometry from the tracked record, exactly-one `@content` when a template is
/// configured, and the page's PRIMARY pagination output for paginated tracked
/// pages (reference `Parser::render()`).
pub fn render_tracked(
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    paginate: Option<&crate::project::PaginationConfig>,
) -> Result<RenderResult, RenderError> {
    let content_path = host.content_path(identity);
    let mut state = RenderState {
        pagination_collecting: paginate.is_some(),
        ..RenderState::default()
    };

    let template_path = identity.template_path.clone().unwrap_or_default();
    let output = if template_path.is_empty() {
        if !host.source_readable(&content_path) {
            return Err(RenderError::new(
                ErrorKind::MissingSource,
                format!("content file is not readable: {}", content_path.display()),
            ));
        }
        let content_source = host.read_source(&content_path).map_err(|e| {
            RenderError::new(
                e.kind,
                format!("content file is not readable: {}", content_path.display()),
            )
        })?;
        state
            .input_stack
            .push(content_path.to_string_lossy().to_string());
        let output = parse(
            &mut state,
            host,
            identity,
            None,
            &content_source,
            &content_path,
            0,
        )?;
        state.input_stack.pop();
        state.dependencies.insert(host.relative(&content_path));
        output
    } else {
        let template_path = host.root().join(&template_path);
        if !host.source_exists(&template_path) {
            return Err(RenderError::new(
                ErrorKind::MissingSource,
                format!("template file does not exist: {}", template_path.display()),
            ));
        }
        if !host.source_readable(&template_path) {
            return Err(RenderError::new(
                ErrorKind::MissingSource,
                format!("template file is not readable: {}", template_path.display()),
            ));
        }
        let template_source = host.read_source(&template_path).map_err(|e| {
            RenderError::new(
                e.kind,
                format!("template file is not readable: {}", template_path.display()),
            )
        })?;
        state.dependencies.insert(host.relative(&template_path));
        state
            .input_stack
            .push(template_path.to_string_lossy().to_string());
        let output = parse(
            &mut state,
            host,
            identity,
            Some(&Source::path(&content_path)),
            &template_source,
            &template_path,
            0,
        )?;
        state.input_stack.pop();
        if state.content_count != 1 {
            return Err(RenderError::new(
                ErrorKind::Render,
                "templated tracked items must execute exactly one @content; add @content through the template/input graph or omit the tracked template field",
            ));
        }
        output
    };

    let mut output = output;
    let mut pagination_pages: Vec<String> = Vec::new();
    if let Some(pagination) = paginate {
        pagination_pages = assemble_pagination_pages(
            &mut state,
            host,
            identity,
            &content_path,
            &output,
            pagination,
        )?;
        output = pagination_pages.first().cloned().unwrap_or(output);
    }

    let mut result = RenderResult::new(output);
    // Complete pagination: pages 2..N with page numbers, ascending. Page 1 is
    // `output`.
    if pagination_pages.len() > 1 {
        result.pagination = pagination_pages[1..]
            .iter()
            .enumerate()
            .map(|(index, page_output)| crate::result::PageOutput {
                page: index + 2,
                output: page_output.clone(),
            })
            .collect();
    }
    result.dependencies = state.dependencies;
    result.requirements = state.requirements;
    Ok(result)
}

/// Assembles the PRIMARY pagination page (reference `Parser::render()`
/// pagination tail): exactly one `@paginate`, the pagination template and
/// separator geometry, the page-1 item window and the marker replacement.
fn assemble_pagination_pages(
    state: &mut RenderState,
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    content_path: &Path,
    base_output: &str,
    pagination: &crate::project::PaginationConfig,
) -> Result<Vec<String>, RenderError> {
    // Returns every rendered page in ascending order: index 0 is page 1 (the
    // primary), index N-1 is page N (CP8: complete pagination in the public
    // contract). Mirrors the reference per-page loop.
    if state.paginate_count != 1 {
        return Err(RenderError::new(
            ErrorKind::Render,
            "paginated tracked items must execute exactly one @paginate",
        ));
    }

    let conventional = |suffix: &str| -> PathBuf {
        let parent = content_path.parent().unwrap_or(content_path);
        let stem = content_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        parent.join(format!("{stem}{suffix}.html"))
    };
    let pagination_template = match &pagination.template_path {
        Some(path) => crate::parser::lexically_normal(&host.root().join(path)),
        None => conventional(".paginate"),
    };
    if !path_within(host.root(), &pagination_template)
        || !host.source_readable(&pagination_template)
    {
        return Err(RenderError::new(
            ErrorKind::Render,
            "pagination template is missing, unreadable, or outside the project",
        ));
    }

    let mut separator = PathBuf::new();
    if let Some(separator_path) = &pagination.separator_path {
        separator = crate::parser::lexically_normal(&host.root().join(separator_path));
    } else {
        let candidate = conventional(".separator");
        if host.source_exists(&candidate) {
            separator = candidate;
        }
    }
    if !separator.as_os_str().is_empty()
        && (!path_within(host.root(), &separator) || !host.source_readable(&separator))
    {
        return Err(RenderError::new(
            ErrorKind::Render,
            "pagination separator is unreadable or outside the project",
        ));
    }

    state
        .dependencies
        .insert(host.relative(&pagination_template));
    if !separator.as_os_str().is_empty() {
        state.dependencies.insert(host.relative(&separator));
    }

    let total = std::cmp::max(
        1,
        state
            .pagination_items
            .len()
            .div_ceil(pagination.items_per_page),
    );
    let separator_source = if separator.as_os_str().is_empty() {
        None
    } else {
        host.read_source(&separator)
            .ok()
            .map(|cow| cow.into_owned())
    };
    let page_source = host.read_source(&pagination_template).map_err(|_| {
        RenderError::new(
            ErrorKind::Render,
            "pagination template is missing, unreadable, or outside the project",
        )
    })?;

    let mut pages = Vec::with_capacity(total);
    for page in 1..=total {
        // Item window for this page.
        let begin = (page - 1) * pagination.items_per_page;
        let finish = std::cmp::min(state.pagination_items.len(), begin + pagination.items_per_page);
        let mut items = String::new();
        for index in begin..finish {
            if index != begin {
                if let Some(separator) = &separator_source {
                    items.push_str(separator);
                }
            }
            items.push_str(&state.pagination_items[index]);
        }

        state.pagination_items_text = Some(items);
        state.pagination_current = page;
        state.pagination_total = total;
        state.pagination_context_active = true;
        state.pagination_collecting = false;
        state.pagination_current_output = Some(host.pagination_output_path(identity, page));

        let rendered = parse(
            state,
            host,
            identity,
            Some(&Source::path(content_path)),
            &page_source,
            &pagination_template,
            0,
        )?;
        let mut page_output = base_output.to_string();
        if let Some(marker_position) = page_output.find(PAGINATION_MARKER) {
            page_output.replace_range(
                marker_position..marker_position + PAGINATION_MARKER.len(),
                &rendered,
            );
        }
        pages.push(page_output);
    }
    Ok(pages)
}

/// Per-render mutable state that persists across recursive parses.
struct RenderState {
    content_count: usize,
    html_comment_depth: usize,
    code_block_depth: usize,
    input_stack: Vec<String>,
    /// Scoped value bindings from `@for` loops (and later `@json`), consulted
    /// after host bindings.
    json_bindings: crate::expr::JsonBindings,
    /// Names bound by @json within the current control-block scope stack.
    /// Pop of a scope erases those bindings (reference json_binding_scopes_).
    json_binding_scopes: Vec<Vec<String>>,
    /// Pagination context (NR5 parse surface; active pagination is NR8).
    pagination_current: usize,
    pagination_total: usize,
    pagination_context_active: bool,
    pagination_current_output: Option<PathBuf>,
    /// Whether `@item`/`@paginate` are collecting (rendering the primary
    /// content of a paginated tracked page).
    pagination_collecting: bool,
    /// Rendered `@item` outputs in encounter order (reference
    /// `result_.pagination_items`).
    pagination_items: Vec<String>,
    /// Number of `@paginate` directives executed (exactly one required).
    paginate_count: usize,
    /// The separator-joined rendered items for the page currently being
    /// assembled (reference `pagination_items_text_`; the `paginate.items`
    /// value).
    pagination_items_text: Option<String>,
    /// Dependency spellings discovered during rendering (root-relative).
    dependencies: std::collections::BTreeSet<String>,
    /// Requirement spellings discovered during rendering (root-relative).
    requirements: std::collections::BTreeSet<String>,
}

impl RenderState {
    /// The pagination window exposed to expression evaluation, when active.
    fn pagination_view(&self) -> Option<crate::expr::PaginationView> {
        if self.pagination_context_active {
            Some(crate::expr::PaginationView {
                items: self.pagination_items_text.clone().unwrap_or_default(),
                current: self.pagination_current,
                total: self.pagination_total,
            })
        } else {
            None
        }
    }
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            content_count: 0,
            html_comment_depth: 0,
            code_block_depth: 0,
            input_stack: Vec::new(),
            json_bindings: crate::expr::JsonBindings::new(),
            json_binding_scopes: Vec::new(),
            // The reference defaults the pagination window to page 1 of 1.
            pagination_current: 1,
            pagination_total: 1,
            pagination_context_active: false,
            pagination_current_output: None,
            pagination_collecting: false,
            pagination_items: Vec::new(),
            paginate_count: 0,
            pagination_items_text: None,
            dependencies: std::collections::BTreeSet::new(),
            requirements: std::collections::BTreeSet::new(),
        }
    }
}

/// Parse a control-block body under a fresh @json-binding scope, erasing
/// bindings created inside it when the scope exits (on success AND error).
fn parse_with_scope(
    state: &mut RenderState,
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    page: Option<&Source>,
    body: &str,
    source_path: &Path,
    depth: usize,
) -> Result<String, RenderError> {
    state.json_binding_scopes.push(Vec::new());
    let result = parse(state, host, identity, page, body, source_path, depth);
    if let Some(names) = state.json_binding_scopes.pop() {
        for name in names {
            state.json_bindings.shift_remove(&name);
        }
    }
    result
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
                // Ternary expressions resolve before the ordinary expression
                // path (reference split_ternary).
                match split_ternary(key) {
                    Ok(Some(parts)) => {
                        let rendered = render_ternary(
                            state,
                            host,
                            identity,
                            page,
                            &parts,
                            source_path,
                            depth,
                        )?;
                        output += &rendered;
                        i = end + 1;
                        continue;
                    }
                    Ok(None) => {}
                    Err(()) => {
                        return Err(error_at(
                            ErrorKind::Parse,
                            "ternary expression has '?' without a matching ':'",
                            text,
                            i,
                            source_path,
                        ));
                    }
                }
                match crate::expr::evaluate_expression(
                    &state.json_bindings,
                    host,
                    identity,
                    key,
                    state.pagination_view().as_ref(),
                ) {
                    Ok(value) => {
                        match value {
                            Value::Array(_) | Value::Object(_) => {
                                let kind = if matches!(value, Value::Array(_)) {
                                    "array"
                                } else {
                                    "object"
                                };
                                return Err(error_at(
                                ErrorKind::Render,
                                format!("cannot render JSON {kind} $[{key}]; select an element first"),
                                text,
                                i,
                                source_path,
                            ));
                            }
                            _ => {
                                output += &render_expression_value(&value);
                                i = end + 1;
                                continue;
                            }
                        }
                    }
                    Err(error) => {
                        if error
                            .message
                            .starts_with("unknown value or malformed expression:")
                        {
                            // Unresolvable values fall through to literal
                            // emission, matching the reference.
                        } else {
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
        }

        // @item: outside pagination collection it is a controlled error; while
        // rendering a paginated tracked page it collects the rendered body as a
        // pagination item (reference Parser @item).
        if text[i..].starts_with("@item")
            && (i + 5 == len
                || matches!(text.as_bytes()[i + 5], b'{')
                || text.as_bytes()[i + 5].is_ascii_whitespace())
        {
            if !state.pagination_collecting {
                return Err(error_at(
                    ErrorKind::Render,
                    "@item requires pagination on the tracked item",
                    text,
                    i,
                    source_path,
                ));
            }
            let mut block_open = i + 5;
            while block_open < len && text.as_bytes()[block_open].is_ascii_whitespace() {
                block_open += 1;
            }
            if block_open >= len || text.as_bytes()[block_open] != b'{' {
                return Err(error_at(
                    ErrorKind::Parse,
                    "@item must be followed by a '{...}' block",
                    text,
                    i,
                    source_path,
                ));
            }
            let block_close = find_balanced(text, block_open, b'{', b'}').ok_or_else(|| {
                error_at(
                    ErrorKind::Parse,
                    "@item block has no matching '}'",
                    text,
                    block_open,
                    source_path,
                )
            })?;
            let body = normalize_control_block_body(&text[block_open + 1..block_close]);
            let nested = parse_with_scope(
                state,
                host,
                identity,
                page,
                &body.text,
                source_path,
                depth + 1,
            )?;
            state.pagination_items.push(nested);
            i = block_close + 1;
            continue;
        }
        // @paginate: emits the pagination marker; exactly one is allowed per
        // paginated tracked render.
        if text[i..].starts_with("@paginate")
            && (i + 9 == len
                || !(text.as_bytes()[i + 9].is_ascii_alphanumeric()
                    || text.as_bytes()[i + 9] == b'_'))
        {
            if !state.pagination_collecting {
                return Err(error_at(
                    ErrorKind::Render,
                    "@paginate requires pagination on the tracked item",
                    text,
                    i,
                    source_path,
                ));
            }
            state.paginate_count += 1;
            if state.paginate_count > 1 {
                return Err(error_at(
                    ErrorKind::Render,
                    "paginated tracked items must execute exactly one @paginate",
                    text,
                    i,
                    source_path,
                ));
            }
            output.push_str(PAGINATION_MARKER);
            i += 9;
            continue;
        }

        // @for(...){...} over arrays or objects.
        if text[i..].starts_with("@for(") {
            let header_close = find_balanced(text, i + 4, b'(', b')').ok_or_else(|| {
                error_at(
                    ErrorKind::Parse,
                    "@for has no matching ')' for its header",
                    text,
                    i,
                    source_path,
                )
            })?;
            let mut block_open = header_close + 1;
            while block_open < len && matches!(bytes[block_open], b' ' | b'\t' | b'\r' | b'\n') {
                block_open += 1;
            }
            if block_open >= len || bytes[block_open] != b'{' {
                return Err(error_at(
                    ErrorKind::Parse,
                    "@for(...) must be followed by a '{...}' block",
                    text,
                    i,
                    source_path,
                ));
            }
            let block_close = find_balanced(text, block_open, b'{', b'}').ok_or_else(|| {
                error_at(
                    ErrorKind::Parse,
                    "@for block has no matching '}'",
                    text,
                    block_open,
                    source_path,
                )
            })?;

            let header = text[i + 5..header_close].trim();
            let separator = crate::expr::find_top_level(header, ":");
            let Some(separator) = separator else {
                return Err(error_at(
                    ErrorKind::Parse,
                    "@for header must contain ':'",
                    text,
                    i,
                    source_path,
                ));
            };
            let binding_part = header[..separator].trim();
            let collection_clause = header[separator + 1..].trim();
            let (collection_expression, sort_expression, sort_descending) =
                parse_for_collection_clause(collection_clause)?;

            let pagination = state.pagination_view();
            let collection_value = crate::expr::evaluate_collection_value(
                &mut state.json_bindings,
                host,
                identity,
                &collection_expression,
                pagination.as_ref(),
            )
            .map_err(|e| {
                // Reference wraps @for collection failures with the directive
                // context.
                error_at(
                    ErrorKind::Render,
                    format!("@for collection: {}", e.message),
                    text,
                    i,
                    source_path,
                )
            })?;

            let body = normalize_control_block_body(&text[block_open + 1..block_close]);
            let body_multiline = body.multiline;
            let control_indent = insertion_indent(&output);
            let insertion_code_block_depth = state.code_block_depth;

            match &collection_value {
                Value::Array(array) => {
                    if !crate::bindings::valid_binding_identifier(binding_part) {
                        return Err(error_at(
                            ErrorKind::Parse,
                            "array @for syntax is @for(item : array){...}",
                            text,
                            i,
                            source_path,
                        ));
                    }
                    if crate::expr::reserved_binding_name(binding_part) {
                        return Err(error_at(
                            ErrorKind::Parse,
                            format!(
                                "@for binding '{binding_part}' conflicts with built-in metadata"
                            ),
                            text,
                            i,
                            source_path,
                        ));
                    }
                    if !sort_expression.is_empty()
                        && !(sort_expression == binding_part
                            || sort_expression.starts_with(&format!("{binding_part}."))
                            || sort_expression.starts_with(&format!("{binding_part}[")))
                    {
                        return Err(error_at(
                            ErrorKind::Parse,
                            format!(
                                "@for array sort key must begin with loop binding '{binding_part}'"
                            ),
                            text,
                            i,
                            source_path,
                        ));
                    }

                    let mut order: Vec<usize> = (0..array.len()).collect();
                    if !sort_expression.is_empty() {
                        let mut sort_keys: Vec<Value> = Vec::new();
                        let mut key_type: Option<std::mem::Discriminant<Value>> = None;
                        for element in array {
                            let prior = state.json_bindings.get(binding_part).cloned();
                            state
                                .json_bindings
                                .insert(binding_part.to_string(), element.clone());
                            let key_result = crate::expr::resolve_json_value(
                                &state.json_bindings,
                                host,
                                &sort_expression,
                            );
                            match prior {
                                Some(prior) => {
                                    state.json_bindings.insert(binding_part.to_string(), prior)
                                }
                                None => state.json_bindings.shift_remove(binding_part),
                            };
                            let key = match key_result {
                                Ok(Some(key)) => key,
                                Ok(None) => {
                                    return Err(error_at(
                                        ErrorKind::Render,
                                        "@for sort key is not a bound JSON path",
                                        text,
                                        i,
                                        source_path,
                                    ));
                                }
                                Err(error) => {
                                    return Err(error_at(
                                        ErrorKind::Render,
                                        error.message,
                                        text,
                                        i,
                                        source_path,
                                    ));
                                }
                            };
                            if !key.is_number() && !key.is_string() {
                                return Err(error_at(
                                    ErrorKind::Render,
                                    "@for sort keys must all be numbers or all be strings",
                                    text,
                                    i,
                                    source_path,
                                ));
                            }
                            match key_type {
                                None => key_type = Some(std::mem::discriminant(&key)),
                                Some(t) if t != std::mem::discriminant(&key) => {
                                    return Err(error_at(
                                        ErrorKind::Render,
                                        "@for sort keys must have the same type",
                                        text,
                                        i,
                                        source_path,
                                    ));
                                }
                                Some(_) => {}
                            }
                            sort_keys.push(key);
                        }
                        order.sort_by(|&a, &b| {
                            let ordering = sort_key_compare(&sort_keys[a], &sort_keys[b]);
                            if sort_descending {
                                ordering.reverse()
                            } else {
                                ordering
                            }
                        });
                    }

                    let prior_element = state.json_bindings.get(binding_part).cloned();
                    let prior_loop = state.json_bindings.get("loop").cloned();
                    let mut iteration_error: Option<RenderError> = None;
                    for (position, &index) in order.iter().enumerate() {
                        let element = &array[index];
                        state
                            .json_bindings
                            .insert(binding_part.to_string(), element.clone());
                        state.json_bindings.insert(
                            "loop".to_string(),
                            make_loop_metadata(position, array.len()),
                        );
                        match parse_with_scope(
                            state,
                            host,
                            identity,
                            page,
                            &body.text,
                            source_path,
                            depth + 1,
                        ) {
                            Ok(nested) => {
                                append_indented(
                                    &mut output,
                                    &nested,
                                    &control_indent,
                                    insertion_code_block_depth,
                                );
                                if body_multiline && position + 1 < order.len() {
                                    output.push('\n');
                                    output.push_str(&control_indent);
                                }
                            }
                            Err(error) => {
                                iteration_error = Some(error);
                                break;
                            }
                        }
                    }
                    restore_binding(&mut state.json_bindings, binding_part, prior_element);
                    restore_binding(&mut state.json_bindings, "loop", prior_loop);
                    if let Some(error) = iteration_error {
                        return Err(error_at(
                            ErrorKind::Render,
                            error.message,
                            text,
                            i,
                            source_path,
                        ));
                    }
                    i = block_close + 1;
                    continue;
                }
                Value::Object(object) => {
                    if binding_part.len() < 5
                        || !binding_part.starts_with('(')
                        || !binding_part.ends_with(')')
                    {
                        return Err(error_at(
                            ErrorKind::Parse,
                            "object @for syntax is @for((key, val) : object){...}",
                            text,
                            i,
                            source_path,
                        ));
                    }
                    let pair = &binding_part[1..binding_part.len() - 1];
                    let Some(comma) = pair.find(',') else {
                        return Err(error_at(
                            ErrorKind::Parse,
                            "object @for requires exactly two bindings: (key, val)",
                            text,
                            i,
                            source_path,
                        ));
                    };
                    if pair[comma + 1..].contains(',') {
                        return Err(error_at(
                            ErrorKind::Parse,
                            "object @for requires exactly two bindings: (key, val)",
                            text,
                            i,
                            source_path,
                        ));
                    }
                    let key_name = pair[..comma].trim();
                    let value_name = pair[comma + 1..].trim();
                    if !crate::bindings::valid_binding_identifier(key_name)
                        || !crate::bindings::valid_binding_identifier(value_name)
                        || key_name == value_name
                    {
                        return Err(error_at(
                            ErrorKind::Parse,
                            "object @for key and value bindings must be distinct identifiers",
                            text,
                            i,
                            source_path,
                        ));
                    }
                    if crate::expr::reserved_binding_name(key_name)
                        || crate::expr::reserved_binding_name(value_name)
                    {
                        return Err(error_at(
                            ErrorKind::Parse,
                            "@for bindings cannot conflict with built-in metadata",
                            text,
                            i,
                            source_path,
                        ));
                    }
                    let valid_object_sort_root = sort_expression.is_empty()
                        || sort_expression == key_name
                        || sort_expression == value_name
                        || sort_expression.starts_with(&format!("{key_name}."))
                        || sort_expression.starts_with(&format!("{key_name}["))
                        || sort_expression.starts_with(&format!("{value_name}."))
                        || sort_expression.starts_with(&format!("{value_name}["));
                    if !valid_object_sort_root {
                        return Err(error_at(
                            ErrorKind::Parse,
                            format!(
                                "@for object sort key must begin with key/value binding '{key_name}' or '{value_name}'"
                            ),
                            text,
                            i,
                            source_path,
                        ));
                    }
                    let prior_key = state.json_bindings.get(key_name).cloned();
                    let prior_value = state.json_bindings.get(value_name).cloned();
                    let prior_loop = state.json_bindings.get("loop").cloned();

                    let mut order: Vec<usize> = (0..object.len()).collect();
                    if !sort_expression.is_empty() {
                        let mut sort_keys: Vec<Value> = Vec::new();
                        let mut key_type: Option<std::mem::Discriminant<Value>> = None;
                        let mut sort_error: Option<RenderError> = None;
                        for (n, (object_key, object_value)) in object.iter().enumerate() {
                            state
                                .json_bindings
                                .insert(key_name.to_string(), Value::string(object_key.clone()));
                            state
                                .json_bindings
                                .insert(value_name.to_string(), object_value.clone());
                            let key_result = crate::expr::resolve_json_value(
                                &state.json_bindings,
                                host,
                                &sort_expression,
                            );
                            state.json_bindings.insert(
                                key_name.to_string(),
                                prior_key.clone().unwrap_or(Value::null()),
                            );
                            state.json_bindings.insert(
                                value_name.to_string(),
                                prior_value.clone().unwrap_or(Value::null()),
                            );
                            let key = match key_result {
                                Ok(Some(key)) => key,
                                Ok(None) => {
                                    sort_error = Some(RenderError::new(
                                        ErrorKind::Render,
                                        format!(
                                            "@for sort key is not a bound JSON path: {sort_expression}"
                                        ),
                                    ));
                                    break;
                                }
                                Err(error) => {
                                    sort_error = Some(error);
                                    break;
                                }
                            };
                            if !key.is_number() && !key.is_string() {
                                sort_error = Some(RenderError::new(
                                    ErrorKind::Render,
                                    format!(
                                        "@for sort keys must all be numbers or all be strings: {sort_expression}"
                                    ),
                                ));
                                break;
                            }
                            match key_type {
                                None => key_type = Some(std::mem::discriminant(&key)),
                                Some(t) if t != std::mem::discriminant(&key) => {
                                    sort_error = Some(RenderError::new(
                                        ErrorKind::Render,
                                        format!(
                                            "@for sort keys must have the same type: {sort_expression}"
                                        ),
                                    ));
                                    break;
                                }
                                Some(_) => {}
                            }
                            sort_keys.push(key);
                            let _ = n;
                        }
                        if let Some(error) = sort_error {
                            restore_binding(&mut state.json_bindings, key_name, prior_key.clone());
                            restore_binding(
                                &mut state.json_bindings,
                                value_name,
                                prior_value.clone(),
                            );
                            return Err(error_at(
                                ErrorKind::Render,
                                error.message,
                                text,
                                i,
                                source_path,
                            ));
                        }
                        order.sort_by(|&a, &b| {
                            let ordering = sort_key_compare(&sort_keys[a], &sort_keys[b]);
                            if sort_descending {
                                ordering.reverse()
                            } else {
                                ordering
                            }
                        });
                    }

                    let mut iteration_error: Option<RenderError> = None;
                    for (position, &index) in order.iter().enumerate() {
                        let (object_key, object_value) = object.get_index(index).unwrap();
                        state
                            .json_bindings
                            .insert(key_name.to_string(), Value::string(object_key.clone()));
                        state
                            .json_bindings
                            .insert(value_name.to_string(), object_value.clone());
                        state.json_bindings.insert(
                            "loop".to_string(),
                            make_loop_metadata(position, object.len()),
                        );
                        match parse_with_scope(
                            state,
                            host,
                            identity,
                            page,
                            &body.text,
                            source_path,
                            depth + 1,
                        ) {
                            Ok(nested) => {
                                append_indented(
                                    &mut output,
                                    &nested,
                                    &control_indent,
                                    insertion_code_block_depth,
                                );
                                if body_multiline && position + 1 < order.len() {
                                    output.push('\n');
                                    output.push_str(&control_indent);
                                }
                            }
                            Err(error) => {
                                iteration_error = Some(error);
                                break;
                            }
                        }
                    }
                    restore_binding(&mut state.json_bindings, key_name, prior_key);
                    restore_binding(&mut state.json_bindings, value_name, prior_value);
                    restore_binding(&mut state.json_bindings, "loop", prior_loop);
                    if let Some(error) = iteration_error {
                        return Err(error_at(
                            ErrorKind::Render,
                            error.message,
                            text,
                            i,
                            source_path,
                        ));
                    }
                    i = block_close + 1;
                    continue;
                }
                _ => {
                    return Err(error_at(
                        ErrorKind::Render,
                        "@for can only iterate over JSON arrays or objects",
                        text,
                        i,
                        source_path,
                    ));
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

            let condition_value = crate::expr::evaluate_expression(
                &state.json_bindings,
                host,
                identity,
                &text[i + 4..condition_close],
                state.pagination_view().as_ref(),
            )
            .map(|value| truthy(&value))
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
                let nested = parse_with_scope(
                    state,
                    host,
                    identity,
                    page,
                    &body.text,
                    source_path,
                    depth + 1,
                )?;
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
                        branch_condition = crate::expr::evaluate_expression(
                            &state.json_bindings,
                            host,
                            identity,
                            &text[cursor + 1..else_condition_close],
                            state.pagination_view().as_ref(),
                        )
                        .map(|value| truthy(&value))
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
                    let nested = parse_with_scope(
                        state,
                        host,
                        identity,
                        page,
                        &body.text,
                        source_path,
                        depth + 1,
                    )?;
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
            let mut parameters: Vec<String> = Vec::new();
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
                parameters = crate::expr::parse_parameters(&text[name_end + 1..close]);
                end = close + 1;
            }
            let parameters_count = parameters.len();

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
                let (content_text, content_identity) = match resolve_source(host, page) {
                    Ok(pair) => pair,
                    Err(_) if matches!(page, Source::Path(_)) => {
                        // Reference message for a missing/unreadable content
                        // page (the corpus missing-source reject class).
                        return Err(error_at(
                            ErrorKind::MissingSource,
                            "content file is not readable",
                            text,
                            i,
                            source_path,
                        ));
                    }
                    Err(e) => {
                        return Err(error_at(e.kind, e.message, text, i, source_path));
                    }
                };
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
                );
                if identity_tracked {
                    state.input_stack.pop();
                }
                let nested = nested?;
                // Reference: @content inserts the page through append_indented,
                // which strips the page's single trailing newline and applies
                // the current indentation.
                let insertion_code_block_depth = state.code_block_depth;
                let indent = insertion_indent(&output);
                append_indented(&mut output, &nested, &indent, insertion_code_block_depth);
                if let Source::Path(_) = page {
                    state.dependencies.insert(host.relative(&content_identity));
                }
                i = end;
                continue;
            }

            // Collection operators: render the result as compact JSON.
            if matches!(
                function,
                "filter"
                    | "map"
                    | "sort"
                    | "slice"
                    | "find"
                    | "some"
                    | "every"
                    | "distinct"
                    | "reverse"
                    | "sum"
                    | "prod"
                    | "min"
                    | "max"
                    | "reduce"
            ) {
                if !has_parameters {
                    return Err(error_at(
                        ErrorKind::Parse,
                        format!("{function}: expected parameters"),
                        text,
                        i,
                        source_path,
                    ));
                }
                let call_end = if end > name_end && text.as_bytes()[end - 1] == b';' {
                    end - 1
                } else {
                    end
                };
                let call = format!("@{function}{}", &text[name_end..call_end]);
                let pagination = state.pagination_view();
                let result = crate::expr::evaluate_collection_value(
                    &mut state.json_bindings,
                    host,
                    identity,
                    &call,
                    pagination.as_ref(),
                )
                .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                output += &crate::expr::dump_compact(&result);
                i = end;
                continue;
            }

            if function == "substr" {
                if !has_parameters || parameters_count != 3 {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "substr: expected value, position and length",
                        text,
                        i,
                        source_path,
                    ));
                }
                let value = interpolate_parameter(state, host, identity, &parameters[0])
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                let parse_index = |raw: &str, label: &str| -> Result<usize, RenderError> {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() || trimmed.starts_with('-') {
                        return Err(RenderError::new(
                            ErrorKind::Render,
                            format!("substr: {label} must be a non-negative integer"),
                        ));
                    }
                    trimmed.parse::<usize>().map_err(|_| {
                        RenderError::new(
                            ErrorKind::Render,
                            format!("substr: {label} must be a non-negative integer"),
                        )
                    })
                };
                let position = parse_index(&parameters[1], "position")
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                let length = parse_index(&parameters[2], "length")
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                let chars: Vec<char> = value.chars().collect();
                if position < chars.len() && length > 0 {
                    let finish = (position + length.min(chars.len() - position)).min(chars.len());
                    output.extend(chars[position..finish].iter());
                }
                i = end;
                continue;
            }

            if function == "join" {
                if !has_parameters || parameters_count != 2 {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "join: expected array and separator",
                        text,
                        i,
                        source_path,
                    ));
                }
                let mut expression = parameters[0].trim().to_string();
                if expression.len() >= 3
                    && expression.starts_with("$[")
                    && expression.ends_with(']')
                {
                    expression = expression[2..expression.len() - 1].to_string();
                }
                let pagination = state.pagination_view();
                let array_value = crate::expr::evaluate_collection_value(
                    &mut state.json_bindings,
                    host,
                    identity,
                    &expression,
                    pagination.as_ref(),
                )
                .map_err(|e| {
                    error_at(
                        ErrorKind::Render,
                        format!("join: {}", e.message),
                        text,
                        i,
                        source_path,
                    )
                })?;
                let Value::Array(array) = &array_value else {
                    return Err(error_at(
                        ErrorKind::Render,
                        "join: first parameter must resolve to a JSON array",
                        text,
                        i,
                        source_path,
                    ));
                };
                let separator = interpolate_parameter(state, host, identity, &parameters[1])
                    .map_err(|e| {
                        error_at(
                            ErrorKind::Render,
                            format!("join: {}", e.message),
                            text,
                            i,
                            source_path,
                        )
                    })?;
                for (item_index, item) in array.iter().enumerate() {
                    if item.is_array() || item.is_object() {
                        return Err(error_at(
                            ErrorKind::Render,
                            "join: array items must be scalar JSON values",
                            text,
                            i,
                            source_path,
                        ));
                    }
                    if item_index > 0 {
                        output += &separator;
                    }
                    output += &render_expression_value(item);
                }
                i = end;
                continue;
            }

            if function == "ent" {
                if !has_parameters || parameters_count != 1 {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "@ent expects exactly one entity",
                        text,
                        i,
                        source_path,
                    ));
                }
                let resolved = interpolate_parameter(state, host, identity, &parameters[0])
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                match crate::json::entity(&resolved) {
                    Some(encoded) => output += encoded,
                    None => {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!("do not currently have an entity value for '{resolved}'"),
                            text,
                            i,
                            source_path,
                        ));
                    }
                }
                i = end;
                continue;
            }

            if function == "pathto" || function == "pathtofile" {
                if !has_parameters || parameters_count != 1 {
                    return Err(error_at(
                        ErrorKind::Parse,
                        format!("@{function} expects exactly one path/name"),
                        text,
                        i,
                        source_path,
                    ));
                }
                let resolved = interpolate_parameter(state, host, identity, &parameters[0])
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                if host.tracked_output_path(&resolved).is_none() {
                    let target_path = lexically_normal(&host.root().join(&resolved));
                    if !path_within(host.root(), &target_path) {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!(
                                "@{function} path must stay inside the Nift project: {resolved}"
                            ),
                            text,
                            i,
                            source_path,
                        ));
                    }
                }
                let path_string = path_to(host, identity, &resolved)
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                output += &path_string;
                let requirement = match host.tracked_output_path(&resolved) {
                    Some((path, _)) => path,
                    None => lexically_normal(&host.root().join(&resolved)),
                };
                state.requirements.insert(host.relative(&requirement));
                i = end;
                continue;
            }

            if function == "pathtopage" {
                if !has_parameters || parameters_count != 1 {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "pathtopage: expected 1 page number",
                        text,
                        i,
                        source_path,
                    ));
                }
                let raw = parameters[0].trim();
                let relative = raw.starts_with('+') || raw.starts_with('-');
                let sign: i64 = if raw.starts_with('-') { -1 } else { 1 };
                let page_arg = if relative && raw.len() > 1 {
                    &raw[1..]
                } else {
                    raw
                };
                let resolved = interpolate_parameter(state, host, identity, page_arg)
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                let trimmed = resolved.trim();
                let number: u64 = trimmed.parse().map_err(|_| {
                    error_at(
                        ErrorKind::Parse,
                        "pathtopage: page/offset must be a positive integer",
                        text,
                        i,
                        source_path,
                    )
                })?;
                if number == 0 {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "pathtopage: page/offset must be a positive integer",
                        text,
                        i,
                        source_path,
                    ));
                }
                let mut page: i64 = number as i64;
                if relative {
                    page = state.pagination_current as i64 + sign * (number as i64);
                }
                if page < 1 || (page as u64) > state.pagination_total as u64 {
                    return Err(error_at(
                        ErrorKind::Render,
                        format!(
                            "pathtopage: resolved page must be between 1 and {}",
                            state.pagination_total
                        ),
                        text,
                        i,
                        source_path,
                    ));
                }
                output += &path_to_page(state, host, identity, page as usize)
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                i = end;
                continue;
            }

            if function == "getenv" {
                if !has_parameters || parameters_count != 1 {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "getenv: expected 1 parameter",
                        text,
                        i,
                        source_path,
                    ));
                }
                let resolved = interpolate_parameter(state, host, identity, &parameters[0])
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                match host.environment(&resolved) {
                    crate::host::HostResult::Found(value) => output += &value,
                    crate::host::HostResult::Error(message) => {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!("getenv: {message}"),
                            text,
                            i,
                            source_path,
                        ))
                    }
                    crate::host::HostResult::NotFound => {}
                }
                i = end;
                continue;
            }

            if function == "input" {
                if !has_parameters || parameters_count != 1 {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "input: expected 1 parameter",
                        text,
                        i,
                        source_path,
                    ));
                }
                let resolved = interpolate_parameter(state, host, identity, &parameters[0])
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                let mut input_path = PathBuf::from(&resolved);
                if input_path.is_relative() {
                    let parent = source_path.parent().filter(|p| !p.as_os_str().is_empty());
                    if let Some(parent) = parent {
                        let relative_to_source = parent.join(&input_path);
                        if host.source_exists(&relative_to_source) {
                            input_path = relative_to_source;
                        } else if host.root().as_os_str().is_empty() {
                            return Err(error_at(
                                ErrorKind::Render,
                                format!(
                                    "@input cannot resolve relative path '{resolved}' without a project root"
                                ),
                                text,
                                i,
                                source_path,
                            ));
                        } else {
                            input_path = host.root().join(&input_path);
                        }
                    } else if host.root().as_os_str().is_empty() {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!(
                                "@input cannot resolve relative path '{resolved}' without a project root"
                            ),
                            text,
                            i,
                            source_path,
                        ));
                    } else {
                        input_path = host.root().join(&input_path);
                    }
                }
                if !host.source_exists(&input_path) {
                    return Err(error_at(
                        ErrorKind::Render,
                        format!("@input path does not exist: {resolved}"),
                        text,
                        i,
                        source_path,
                    ));
                }
                let normalized = lexically_normal(
                    &std::path::absolute(&input_path).unwrap_or(input_path.clone()),
                );
                let normalized_string = normalized.to_string_lossy().to_string();
                if state.input_stack.contains(&normalized_string) {
                    return Err(error_at(
                        ErrorKind::Render,
                        format!("@input would result in an input loop through {normalized_string}"),
                        text,
                        i,
                        source_path,
                    ));
                }
                if !host.source_readable(&normalized) {
                    return Err(error_at(
                        ErrorKind::Render,
                        "input file is not readable",
                        text,
                        i,
                        source_path,
                    ));
                }
                state.input_stack.push(normalized_string.clone());
                state.dependencies.insert(host.relative(&normalized));
                let insertion_code_block_depth = state.code_block_depth;
                let input_source = host
                    .read_source(&normalized)
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                let nested = parse(
                    state,
                    host,
                    identity,
                    page,
                    &input_source,
                    &normalized,
                    depth + 1,
                );
                state.input_stack.pop();
                let nested = nested?;
                append_indented(&mut output, &nested, "", insertion_code_block_depth);
                i = end;
                continue;
            }

            if function == "json" {
                if !has_parameters || (parameters_count != 2 && parameters_count != 3) {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "json: expected 2 or 3 parameters (path, name[, schema])",
                        text,
                        i,
                        source_path,
                    ));
                }
                let resolved_path = interpolate_parameter(state, host, identity, &parameters[0])
                    .map_err(|e| error_at(ErrorKind::Render, e.message, text, i, source_path))?;
                let binding_name = parameters[1].trim().to_string();
                if !crate::bindings::valid_binding_identifier(&binding_name) {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "json: name must be an identifier using letters, digits and underscores",
                        text,
                        i,
                        source_path,
                    ));
                }
                if crate::expr::reserved_binding_name(&binding_name) {
                    return Err(error_at(
                        ErrorKind::Parse,
                        format!("json: name '{binding_name}' conflicts with built-in metadata/reserved bindings"),
                        text,
                        i,
                        source_path,
                    ));
                }
                if host.binding(&binding_name).is_some()
                    || state.json_bindings.contains_key(&binding_name)
                {
                    return Err(error_at(
                        ErrorKind::Parse,
                        format!("json: name '{binding_name}' is already bound"),
                        text,
                        i,
                        source_path,
                    ));
                }
                let json_path = lexically_normal(&host.root().join(&resolved_path));
                if !path_within(host.root(), &json_path) {
                    return Err(error_at(
                        ErrorKind::Render,
                        format!("json: path must stay inside the Nift project: {resolved_path}"),
                        text,
                        i,
                        source_path,
                    ));
                }
                if !host.source_exists(&json_path) {
                    return Err(error_at(
                        ErrorKind::Render,
                        format!("json: file does not exist: {resolved_path}"),
                        text,
                        i,
                        source_path,
                    ));
                }
                let document = host.read_json(&json_path).map_err(|e| {
                    error_at(
                        ErrorKind::Render,
                        format!("json: failed to parse {resolved_path} ({})", e.message),
                        text,
                        i,
                        source_path,
                    )
                })?;
                if parameters_count == 3 {
                    let schema_path_argument =
                        interpolate_parameter(state, host, identity, &parameters[2]).map_err(
                            |e| error_at(ErrorKind::Render, e.message, text, i, source_path),
                        )?;
                    let schema_path = lexically_normal(&host.root().join(&schema_path_argument));
                    if !path_within(host.root(), &schema_path) {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!("json: schema path must stay inside the Nift project: {schema_path_argument}"),
                            text,
                            i,
                            source_path,
                        ));
                    }
                    if !host.source_exists(&schema_path) {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!("json: schema file does not exist: {schema_path_argument}"),
                            text,
                            i,
                            source_path,
                        ));
                    }
                    let schema = host.read_json(&schema_path).map_err(|e| {
                        error_at(
                            ErrorKind::Render,
                            format!(
                                "json: failed to parse schema {schema_path_argument} ({})",
                                e.message
                            ),
                            text,
                            i,
                            source_path,
                        )
                    })?;
                    if let Err(validation_error) = crate::json::validate_schema(&document, &schema)
                    {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!(
                                "json: {resolved_path} does not satisfy schema {schema_path_argument} ({validation_error})"
                            ),
                            text,
                            i,
                            source_path,
                        ));
                    }
                    state.dependencies.insert(host.relative(&schema_path));
                }
                if let Some(scope) = state.json_binding_scopes.last_mut() {
                    scope.push(binding_name.clone());
                }
                state.json_bindings.insert(binding_name, document);
                state.dependencies.insert(host.relative(&json_path));
                i = end;
                continue;
            }

            if function == "dep" {
                if !has_parameters || parameters.is_empty() {
                    return Err(error_at(
                        ErrorKind::Parse,
                        "dep: expected parameters",
                        text,
                        i,
                        source_path,
                    ));
                }
                for dependency in &parameters {
                    let resolved = interpolate_parameter(state, host, identity, dependency)
                        .map_err(|e| {
                            error_at(ErrorKind::Render, e.message, text, i, source_path)
                        })?;
                    let dependency_path = lexically_normal(&host.root().join(&resolved));
                    if !path_within(host.root(), &dependency_path) {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!("dep: path must stay inside the Nift project: {resolved}"),
                            text,
                            i,
                            source_path,
                        ));
                    }
                    if !host.source_exists(&dependency_path) {
                        return Err(error_at(
                            ErrorKind::Render,
                            format!("failed as dependency does not exist: {resolved}"),
                            text,
                            i,
                            source_path,
                        ));
                    }
                    state.dependencies.insert(host.relative(&dependency_path));
                }
                i = end;
                continue;
            }

            // Unknown function (or a not-yet-implemented one): literal.
        }

        // Literal fallback. Advance by a full UTF-8 character: a byte-wise
        // advance would corrupt multi-byte characters and land `i` mid-char,
        // panicking on later `text[i..]` slicing (found by the NR10 unicode
        // differential case).
        let ch = text[i..].chars().next().unwrap();
        output.push(ch);
        i += ch.len_utf8();
    }

    Ok(output)
}

/// Scalar rendering: strings verbatim; numbers/bools/null as compact JSON
/// (matching the reference). Number formatting reproduces the reference's
/// `std::to_chars` rules exactly, verified against a differential battery
/// captured from the frozen C++ reference (tests/number_formatting.rs):
/// integer-valued doubles within i64 range render as integers; everything else
/// renders as `to_chars(general, 15)` (fixed notation when the decimal
/// exponent is in [-4, 15), otherwise scientific with a signed two-digit
/// exponent, trailing zeros stripped).
pub(crate) fn format_number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n >= -(2f64.powi(63)) && n < 2f64.powi(63) {
        return format!("{}", n as i64);
    }
    if !n.is_finite() {
        // inf/nan cannot arise from JSON; not part of the reference corpus.
        return format!("{}", n);
    }
    format_general15(n)
}

/// Scalar rendering for `$[...]`: strings verbatim; numbers/bools/null via the
/// reference's compact JSON rules.
fn render_expression_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Number(n) => format_number(*n),
        Value::Null => "null".to_string(),
        Value::Array(_) | Value::Object(_) => unreachable!("handled by the caller"),
    }
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

pub(crate) fn built_in_metadata_name(key: &str) -> bool {
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

pub(crate) fn metadata(
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    key: &str,
) -> Option<String> {
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

pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => *n != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Path spelled with forward slashes (reference generic_string).
/// `std::filesystem::path::generic_string()`: the path spelled with `/`
/// separators regardless of platform. On POSIX the native and generic forms
/// coincide (a literal backslash inside a filename is preserved); on Windows
/// the native `\` separator becomes `/`.
pub(crate) fn generic(path: &Path) -> String {
    let native = path.to_string_lossy().to_string();
    if std::path::MAIN_SEPARATOR == '/' {
        native
    } else {
        native.replace('\\', "/")
    }
}

/// The path/URL emitted by @pathto/@pathtofile (reference `path_to`): current
/// output base, tracked or concrete destination, 404 root-absolute rule,
/// index-page geometry and `./` prefixing.
fn path_to(
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    argument: &str,
) -> Result<String, RenderError> {
    if !host.has_output_context() {
        return Err(RenderError::new(
            ErrorKind::Render,
            format!(
                "@pathto requires a path context; set the current output on the render context to resolve '{argument}'"
            ),
        ));
    }
    let output = host.output_path(identity);
    let base = output.parent().unwrap_or(&output).to_path_buf();
    let absolute_404 = identity.name.as_deref() == Some("404");

    let (destination, index_page) = match host.tracked_output_path(argument) {
        Some((path, index)) => (path, index),
        None => {
            let destination = lexically_normal(&host.root().join(argument));
            if !path_within(host.root(), &destination) {
                return Err(RenderError::new(
                    ErrorKind::Render,
                    format!("pathto: path must stay inside the Nift project: {argument}"),
                ));
            }
            if !host.source_exists(&destination) {
                return Err(RenderError::new(
                    ErrorKind::Render,
                    format!("'{argument}' is neither a tracked name nor a file that exists"),
                ));
            }
            (destination, false)
        }
    };

    if absolute_404 {
        let web_root = lexically_normal(&host.root().join(host.output_dir()));
        let web_rel = generic(&lexically_relative(&destination, &web_root));
        let escapes = web_rel == ".." || web_rel.starts_with("../");
        if !escapes {
            if index_page {
                let dir = destination.parent().unwrap_or(&destination).to_path_buf();
                let dir_rel = generic(&lexically_relative(&dir, &web_root));
                if dir_rel.is_empty() || dir_rel == "." {
                    return Ok("/".to_string());
                }
                let mut path = format!("/{dir_rel}");
                if !path.ends_with('/') {
                    path.push('/');
                }
                return Ok(path);
            }
            return Ok(format!("/{web_rel}"));
        }
    }

    let mut relative = generic(&lexically_relative(&destination, &base));
    if relative.is_empty() {
        relative = generic(&destination);
    }
    if index_page
        && destination
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("index"))
            .unwrap_or(false)
    {
        let parent = destination.parent().unwrap_or(&destination).to_path_buf();
        let mut relative = generic(&lexically_relative(&parent, &base));
        if relative.is_empty() {
            relative = generic(&parent);
        }
        if relative.is_empty() || relative == "." {
            return Ok("./".to_string());
        }
        if !relative.ends_with('/') {
            relative.push('/');
        }
        return Ok(relative);
    }
    if !relative.contains('/') && !relative.starts_with("..") {
        return Ok(format!("./{relative}"));
    }
    Ok(relative)
}

/// The path emitted by @pathtopage (reference `path_to_page`). Requires an
/// active pagination context (NR8); the NR5 guard reports it otherwise.
fn path_to_page(
    state: &RenderState,
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    page: usize,
) -> Result<String, RenderError> {
    if !state.pagination_context_active {
        return Err(RenderError::new(
            ErrorKind::Render,
            "pathtopage: only available while rendering pagination",
        ));
    }
    if page < 1 || page > state.pagination_total {
        return Err(RenderError::new(
            ErrorKind::Render,
            format!(
                "pathtopage: page must be between 1 and {}",
                state.pagination_total
            ),
        ));
    }
    let destination = host.pagination_output_path(identity, page);
    let base = match &state.pagination_current_output {
        Some(out) => out.parent().unwrap_or(out).to_path_buf(),
        None => {
            let output = host.output_path(identity);
            output.parent().unwrap_or(&output).to_path_buf()
        }
    };
    if page == 1
        && (identity.name.as_deref() == Some("/")
            || identity
                .name
                .as_deref()
                .map(|n| n.ends_with('/'))
                .unwrap_or(false))
    {
        let parent = destination.parent().unwrap_or(&destination).to_path_buf();
        let mut relative = generic(&lexically_relative(&parent, &base));
        if relative.is_empty() {
            relative = generic(&parent);
        }
        if relative.is_empty() || relative == "." {
            return Ok("./".to_string());
        }
        if !relative.ends_with('/') {
            relative.push('/');
        }
        return Ok(relative);
    }
    let mut relative = generic(&lexically_relative(&destination, &base));
    if relative.is_empty() {
        relative = generic(&destination);
    }
    if !relative.contains('/') && !relative.starts_with("..") {
        return Ok(format!("./{relative}"));
    }
    Ok(relative)
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

/// Lexical relative path of `path` against `base` (mirroring the reference's
/// `lexically_relative`), used by containment checks.
/// `std::filesystem::path::lexically_relative()`. Returns `.` when the paths
/// are equivalent (the C++ reference returns `.`, not an empty path, for equal
/// paths); the index-page `@pathto` branch depends on that exact result.
fn lexically_relative(path: &Path, base: &Path) -> PathBuf {
    let path_components: Vec<Component> = path.components().collect();
    let base_components: Vec<Component> = base.components().collect();
    let common = path_components
        .iter()
        .zip(&base_components)
        .take_while(|(a, b)| a == b)
        .count();
    let mut result = PathBuf::new();
    for _ in common..base_components.len() {
        result.push("..");
    }
    for component in &path_components[common..] {
        result.push(component.as_os_str());
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

/// Whether `candidate` stays lexically inside `base` (reference
/// `path_within`): the candidate's relative path must not escape via `..`.
pub(crate) fn path_within(base: &Path, candidate: &Path) -> bool {
    let base_norm = lexically_normal(base);
    let cand_norm = lexically_normal(candidate);
    let rel = lexically_relative(&cand_norm, &base_norm);
    rel.components().next() != Some(Component::ParentDir)
}

/// Lexical path normalisation (resolve `.`/`..` without touching the
/// filesystem), matching the reference's `lexically_normal` for the absolute
/// paths used as render identities.
pub(crate) fn lexically_normal(path: &Path) -> PathBuf {
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

pub(crate) fn find_balanced(
    text: &str,
    open: usize,
    open_char: u8,
    close_char: u8,
) -> Option<usize> {
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

/// A `$[...]` ternary split, mirroring the reference `split_ternary`.
struct TernaryParts<'a> {
    condition: &'a str,
    when_true: &'a str,
    when_false: &'a str,
}

/// Split a `$[...]` key into a ternary. `Ok(Some)` when a top-level `?` is
/// present (with a matching `:` or, when the condition is non-empty, an
/// implicit empty false branch); `Err(())` for a malformed `?` with an empty
/// condition and no `:`; `Ok(None)` when there is no ternary.
fn split_ternary(expression: &str) -> Result<Option<TernaryParts<'_>>, ()> {
    let bytes = expression.as_bytes();
    let mut quoted = false;
    let mut quote = 0u8;
    let mut parens = 0;
    let mut brackets = 0;
    let mut braces = 0;
    let mut question: Option<usize> = None;
    let mut nested = 0;
    let mut i = 0;
    while i < expression.len() {
        let c = bytes[i];
        if quoted {
            if c == b'\\' && i + 1 < expression.len() {
                i += 1;
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
        if c == b'(' {
            parens += 1;
            i += 1;
            continue;
        }
        if c == b')' {
            if parens > 0 {
                parens -= 1;
            }
            i += 1;
            continue;
        }
        if c == b'[' {
            brackets += 1;
            i += 1;
            continue;
        }
        if c == b']' {
            if brackets > 0 {
                brackets -= 1;
            }
            i += 1;
            continue;
        }
        if c == b'{' {
            braces += 1;
            i += 1;
            continue;
        }
        if c == b'}' {
            if braces > 0 {
                braces -= 1;
            }
            i += 1;
            continue;
        }
        if parens > 0 || brackets > 0 || braces > 0 {
            i += 1;
            continue;
        }
        if c == b'?' {
            if question.is_none() {
                question = Some(i);
            } else {
                nested += 1;
            }
            i += 1;
            continue;
        }
        if let Some(q) = question {
            if c == b':' {
                if nested > 0 {
                    nested -= 1;
                    i += 1;
                    continue;
                }
                return Ok(Some(TernaryParts {
                    condition: expression[..q].trim(),
                    when_true: &expression[q + 1..i],
                    when_false: &expression[i + 1..],
                }));
            }
        }
        i += 1;
    }
    if let Some(q) = question {
        let condition = expression[..q].trim();
        if condition.is_empty() {
            return Err(());
        }
        return Ok(Some(TernaryParts {
            condition,
            when_true: &expression[q + 1..],
            when_false: "",
        }));
    }
    Ok(None)
}

/// A quoted string literal (`'...'` or `"..."`) with the reference escape set;
/// `None` when the text is not a quoted string.
fn quoted_string_literal(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let quote = bytes[0];
    if (quote != b'\'' && quote != b'"') || bytes[bytes.len() - 1] != quote {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let mut result = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('\'') => result.push('\''),
                Some(other) => result.push(other),
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    Some(result)
}

/// Renders a `$[...]` ternary branch (reference Parser `$[` ternary path):
/// the condition is a hard-evaluated boolean expression, the selected branch is
/// a quoted string literal (emitted as its value) or lazy Nift source (parsed
/// so a branch can contain directives such as `@input`).
fn render_ternary(
    state: &mut RenderState,
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    page: Option<&Source>,
    parts: &TernaryParts<'_>,
    source_path: &Path,
    depth: usize,
) -> Result<String, RenderError> {
    let condition_value = crate::expr::evaluate_expression(
        &state.json_bindings,
        host,
        identity,
        parts.condition,
        state.pagination_view().as_ref(),
    )?;
    let selected = if truthy(&condition_value) {
        parts.when_true
    } else {
        parts.when_false
    };
    if let Some(literal) = quoted_string_literal(selected.trim()) {
        return Ok(literal);
    }
    parse(
        state,
        host,
        identity,
        page,
        selected,
        source_path,
        depth + 1,
    )
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
        // Advance by a full UTF-8 character (a byte-wise advance lands mid-char
        // on multi-byte text, panicking on later `text[i..]` slicing).
        i += text[i..].chars().next().unwrap().len_utf8();
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
/// output indentation). `multiline` records whether the block used the usual
/// structural form (leading newline inside the braces); the reference uses it
/// to decide whether `@for` iterations are joined by a newline.
struct NormalizedBody {
    text: String,
    multiline: bool,
}

fn normalize_control_block_body(body: &str) -> NormalizedBody {
    let mut body = body.to_string();
    let bytes_len = |s: &str| s.len();
    let mut multiline = false;
    let mut first = 0;
    while first < body.len() && matches!(body.as_bytes()[first], b' ' | b'\t') {
        first += 1;
    }
    if first < body.len() && matches!(body.as_bytes()[first], b'\n' | b'\r') {
        multiline = true;
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
    NormalizedBody {
        text: body,
        multiline,
    }
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

/// Order two `@for` sort keys (numbers or strings).
fn sort_key_compare(left: &Value, right: &Value) -> std::cmp::Ordering {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    }
}

/// Restore a scoped binding to its prior value (or remove it).
fn restore_binding(bindings: &mut crate::expr::JsonBindings, key: &str, prior: Option<Value>) {
    match prior {
        Some(value) => bindings.insert(key.to_string(), value),
        None => bindings.shift_remove(key),
    };
}

/// `@for` collection clause: `collection [by key asc|desc]`, with top-level
/// `by` detection (reference `parse_for_collection_clause`).
fn parse_for_collection_clause(clause: &str) -> Result<(String, String, bool), RenderError> {
    let mut quoted = false;
    let mut quote = 0u8;
    let mut parens = 0;
    let mut brackets = 0;
    let bytes = clause.as_bytes();
    let mut i = 0;
    while i + 4 <= clause.len() {
        let c = bytes[i];
        if quoted {
            if c == b'\\' && i + 1 < clause.len() {
                i += 1;
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
        match c {
            b'(' => parens += 1,
            b')' => {
                if parens > 0 {
                    parens -= 1;
                }
            }
            b'[' => brackets += 1,
            b']' if brackets > 0 => brackets -= 1,
            _ => {}
        }
        if parens == 0
            && brackets == 0
            && clause.is_char_boundary(i)
            && clause[i..].starts_with(" by ")
        {
            let collection = clause[..i].trim().to_string();
            let tail = &clause[i + 4..];
            let space = tail.rfind([' ', '\t']);
            let Some(space) = space else {
                return Err(RenderError::new(
                    ErrorKind::Render,
                    "@for sorting syntax is @for(item : collection by item.field asc|desc){...}",
                ));
            };
            let sort_expression = tail[..space].trim().to_string();
            let direction = tail[space + 1..].trim();
            if direction != "asc" && direction != "desc" {
                return Err(RenderError::new(
                    ErrorKind::Render,
                    "@for sorting syntax is @for(item : collection by item.field asc|desc){...}",
                ));
            }
            return Ok((collection, sort_expression, direction == "desc"));
        }
        i += 1;
    }
    Ok((clause.trim().to_string(), String::new(), false))
}

/// The `loop` metadata object injected by `@for`.
fn make_loop_metadata(index: usize, length: usize) -> Value {
    let mut loop_value = Value::object();
    let _ = loop_value.insert("index", Value::number((index + 1) as f64));
    let _ = loop_value.insert("index0", Value::number(index as f64));
    let _ = loop_value.insert("first", Value::boolean(index == 0));
    let _ = loop_value.insert("last", Value::boolean(index + 1 == length));
    let _ = loop_value.insert("length", Value::number(length as f64));
    loop_value
}

/// Interpolate `$[...]` expressions inside a parameter string
/// (reference `interpolate_parameter`).
fn interpolate_parameter(
    state: &RenderState,
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    text: &str,
) -> Result<String, RenderError> {
    let mut output = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("$[") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match crate::expr::scan_balanced_bracket(after) {
            Some(end_rel) => {
                let key = &after[..end_rel];
                match crate::expr::evaluate_expression(
                    &state.json_bindings,
                    host,
                    identity,
                    key,
                    state.pagination_view().as_ref(),
                ) {
                    Ok(value) => match value {
                        Value::Array(_) | Value::Object(_) => {
                            return Err(RenderError::new(
                                ErrorKind::Render,
                                "cannot interpolate JSON collection into a string".to_string(),
                            ));
                        }
                        _ => output.push_str(&render_expression_value(&value)),
                    },
                    Err(error) => {
                        if error
                            .message
                            .starts_with("unknown value or malformed expression:")
                        {
                            output.push_str(&rest[..start]);
                            output.push_str("$[");
                            rest = after;
                            continue;
                        }
                        return Err(error);
                    }
                }
                rest = &after[end_rel + 1..];
            }
            None => {
                output.push_str(&rest[..start]);
                output.push_str("$[");
                rest = after;
            }
        }
    }
    output.push_str(rest);
    Ok(output)
}
