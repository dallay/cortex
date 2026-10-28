//! Syntax-only Rust navigation. Byte ranges are half-open; lines and Unicode
//! scalar columns are one-based. External modules and macros are never opened.
use proc_macro2::Span;
use serde_json::{json, Value};
use syn::spanned::Spanned;

pub const MAX_SOURCE_BYTES: usize = 1_048_576;
pub const MAX_OUTPUT_BYTES: usize = 49_152;
pub const MAX_DECLARATIONS: usize = 1_000;

pub fn outline(source: &str) -> Value {
    if source.len() > MAX_SOURCE_BYTES {
        return failure("source exceeds 1 MiB");
    }
    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => return failure(&format!("syntax error: {error}")),
    };
    let mut collector = Collector {
        source,
        offset: usize::from(source.starts_with('\u{feff}')) * '\u{feff}'.len_utf8()
            + file.shebang.as_ref().map_or(0, String::len),
        entries: Vec::new(),
        endpoints: Vec::new(),
        bytes: 0,
        limited: false,
    };
    collector.items(&file.items, None);
    collector.fill_positions();
    json!({
        "complete": !collector.limited,
        "declarations": collector.entries,
        "diagnostics": if collector.limited { vec!["declaration or output limit reached"] } else { vec![] },
    })
}

fn failure(message: &str) -> Value {
    json!({"complete":false,"declarations":[],"diagnostics":[message]})
}

struct Collector<'a> {
    source: &'a str,
    offset: usize,
    entries: Vec<Value>,
    endpoints: Vec<(usize, usize, bool)>,
    bytes: usize,
    limited: bool,
}

impl Collector<'_> {
    fn fill_positions(&mut self) {
        // Parent ends can precede child starts in collection order. Sort the
        // bounded endpoints and advance through disjoint source segments.
        self.endpoints.sort_unstable();
        let (mut cursor, mut line, mut column) = (0, 1, 1);
        for &(offset, index, start) in &self.endpoints {
            for segment in self.source[cursor..offset].split_inclusive('\n') {
                if segment.ends_with('\n') {
                    line += 1;
                    column = 1;
                } else {
                    column += segment.chars().count();
                }
            }
            cursor = offset;
            let range = &mut self.entries[index]["range"];
            let (line_key, column_key) = if start {
                ("start_line", "start_column")
            } else {
                ("end_line", "end_column")
            };
            range[line_key] = json!(line);
            range[column_key] = json!(column);
        }
    }

    fn add(
        &mut self,
        name: &str,
        kind: &str,
        container: Option<&str>,
        span: Span,
        header_end: usize,
    ) {
        if self.limited {
            return;
        }
        let raw = span.byte_range();
        let range = raw.start + self.offset..raw.end + self.offset;
        let Some(body) = self.source.get(range.clone()) else {
            self.limited = true;
            return;
        };
        let Some(signature) = self
            .source
            .get(range.start..(header_end + self.offset).min(range.end))
        else {
            self.limited = true;
            return;
        };
        if body.is_empty() || signature.len() > MAX_OUTPUT_BYTES / 2 {
            self.limited = true;
            return;
        }
        // Reserve the maximum coordinate width before accepting a record;
        // filling its actual positions must never increase the output size.
        let coordinate_bound = MAX_SOURCE_BYTES + 1;
        let entry = json!({"name":name,"kind":kind,"container":container,
            "signature":signature.trim_end().trim_end_matches(';').trim_end(),
            "range":{"start_byte":range.start,"end_byte":range.end,
                "start_line":coordinate_bound,"start_column":coordinate_bound,
                "end_line":coordinate_bound,"end_column":coordinate_bound}});
        let cost = entry.to_string().len() + 1;
        if self.entries.len() == MAX_DECLARATIONS || self.bytes + cost > MAX_OUTPUT_BYTES - 512 {
            self.limited = true;
            return;
        }
        self.bytes += cost;
        let index = self.entries.len();
        self.endpoints.push((range.start, index, true));
        self.endpoints.push((range.end, index, false));
        self.entries.push(entry);
    }

    fn items(&mut self, items: &[syn::Item], container: Option<&str>) {
        for item in items {
            if self.limited {
                break;
            }
            let span = item.span();
            match item {
                syn::Item::Fn(value) => self.add(
                    &value.sig.ident.to_string(),
                    "function",
                    container,
                    span,
                    value.block.brace_token.span.open().byte_range().start,
                ),
                syn::Item::Struct(value) => {
                    let end = match &value.fields {
                        syn::Fields::Named(fields) => {
                            fields.brace_token.span.open().byte_range().start
                        }
                        _ => span.byte_range().end,
                    };
                    self.add(&value.ident.to_string(), "struct", container, span, end);
                }
                syn::Item::Enum(value) => self.add(
                    &value.ident.to_string(),
                    "enum",
                    container,
                    span,
                    value.brace_token.span.open().byte_range().start,
                ),
                syn::Item::Mod(value) => {
                    let name = value.ident.to_string();
                    let end = value.content.as_ref().map_or_else(
                        || span.byte_range().end,
                        |(brace, _)| brace.span.open().byte_range().start,
                    );
                    self.add(&name, "module", container, span, end);
                    if let Some((_, items)) = &value.content {
                        let path = container
                            .map_or_else(|| name.clone(), |parent| format!("{parent}::{name}"));
                        self.items(items, Some(&path));
                    }
                }
                syn::Item::Trait(value) => {
                    let name = value.ident.to_string();
                    self.add(
                        &name,
                        "trait",
                        container,
                        span,
                        value.brace_token.span.open().byte_range().start,
                    );
                    let path = container
                        .map_or_else(|| name.clone(), |parent| format!("{parent}::{name}"));
                    for member in &value.items {
                        match member {
                            syn::TraitItem::Fn(method) => {
                                let end = method.default.as_ref().map_or_else(
                                    || method.span().byte_range().end,
                                    |block| block.brace_token.span.open().byte_range().start,
                                );
                                self.add(
                                    &method.sig.ident.to_string(),
                                    "method",
                                    Some(&path),
                                    method.span(),
                                    end,
                                );
                            }
                            syn::TraitItem::Type(value) => self.add(
                                &value.ident.to_string(),
                                "type",
                                Some(&path),
                                value.span(),
                                value.span().byte_range().end,
                            ),
                            syn::TraitItem::Const(value) => self.add(
                                &value.ident.to_string(),
                                "const",
                                Some(&path),
                                value.span(),
                                value.default.as_ref().map_or_else(
                                    || value.span().byte_range().end,
                                    |(eq, _)| eq.span.byte_range().start,
                                ),
                            ),
                            _ => {}
                        }
                    }
                }
                syn::Item::Impl(value) => {
                    let range = value.self_ty.span().byte_range();
                    let name = self
                        .source
                        .get(range.start + self.offset..range.end + self.offset)
                        .unwrap_or("impl")
                        .to_owned();
                    self.add(
                        &name,
                        "impl",
                        container,
                        span,
                        value.brace_token.span.open().byte_range().start,
                    );
                    let path = container
                        .map_or_else(|| name.clone(), |parent| format!("{parent}::{name}"));
                    for member in &value.items {
                        match member {
                            syn::ImplItem::Fn(method) => self.add(
                                &method.sig.ident.to_string(),
                                "method",
                                Some(&path),
                                method.span(),
                                method.block.brace_token.span.open().byte_range().start,
                            ),
                            syn::ImplItem::Type(value) => self.add(
                                &value.ident.to_string(),
                                "type",
                                Some(&path),
                                value.span(),
                                value.span().byte_range().end,
                            ),
                            syn::ImplItem::Const(value) => self.add(
                                &value.ident.to_string(),
                                "const",
                                Some(&path),
                                value.span(),
                                value.eq_token.span.byte_range().start,
                            ),
                            _ => {}
                        }
                    }
                }
                syn::Item::Type(value) => self.add(
                    &value.ident.to_string(),
                    "type",
                    container,
                    span,
                    span.byte_range().end,
                ),
                syn::Item::Const(value) => self.add(
                    &value.ident.to_string(),
                    "const",
                    container,
                    span,
                    value.eq_token.span.byte_range().start,
                ),
                syn::Item::Static(value) => self.add(
                    &value.ident.to_string(),
                    "static",
                    container,
                    span,
                    value.eq_token.span.byte_range().start,
                ),
                syn::Item::Union(value) => self.add(
                    &value.ident.to_string(),
                    "union",
                    container,
                    span,
                    value.fields.brace_token.span.open().byte_range().start,
                ),
                _ => {}
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod component {
    struct Component;
    impl huginn_outline_bindings::Guest for Component {
        fn outline(
            file: &huginn_outline_bindings::huginn::rust_outline::snapshots::Snapshot,
        ) -> String {
            let size = file.size() as usize;
            if size > super::MAX_SOURCE_BYTES {
                return super::failure("source exceeds 1 MiB").to_string();
            }
            let mut bytes = Vec::with_capacity(size);
            while bytes.len() < size {
                let length = (size - bytes.len()).min(65_536) as u32;
                match file.read(bytes.len() as u32, length) {
                    Ok(chunk) if chunk.len() == length as usize => bytes.extend(chunk),
                    _ => return super::failure("snapshot read failed").to_string(),
                }
            }
            String::from_utf8(bytes).map_or_else(
                |_| super::failure("source is not UTF-8").to_string(),
                |source| super::outline(&source).to_string(),
            )
        }
    }
    huginn_outline_bindings::export!(Component with_types_in huginn_outline_bindings);
}
