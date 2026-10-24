//! The symbols in a file, from LSP `textDocument/documentSymbol` (the
//! Swift `DocumentSymbols`): either the hierarchical `DocumentSymbol[]` or
//! the flat `SymbolInformation[]`, flattened in document order with their
//! nesting, for the palette's @ mode and the outline. Also
//! `workspace/symbol` results, each with its file.
//!
//! Positions are 1-based. Anything malformed is skipped rather than failing
//! the whole list.

use serde_json::Value;
use url::Url;

use crate::uri::uri_to_file_path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineSymbol {
    pub name: String,
    pub detail: Option<String>,
    /// LSP SymbolKind (1 File ... 26 TypeParameter).
    pub kind: i64,
    /// 1-based position of the name.
    pub line: usize,
    pub column: usize,
    /// 0 for top level.
    pub depth: usize,
    /// Enclosing symbol names, outermost first.
    pub container: Vec<String>,
}

impl OutlineSymbol {
    pub fn kind_name(&self) -> &'static str {
        kind_name(self.kind)
    }
}

/// A `documentSymbol` result (JSON) as a flat list in document order.
pub fn parse(json: &[u8]) -> Vec<OutlineSymbol> {
    let Ok(Value::Array(array)) = serde_json::from_slice::<Value>(json) else {
        return Vec::new();
    };
    parse_value(&array)
}

/// `parse` for an already decoded result.
pub fn parse_value(array: &[Value]) -> Vec<OutlineSymbol> {
    let mut out = Vec::new();
    if array
        .first()
        .is_some_and(|first| first.get("location").is_some())
    {
        // SymbolInformation: flat, with an optional container name.
        for item in array {
            let (Some(name), Some(kind)) = (name_of(item), kind_of(item)) else {
                continue;
            };
            let Some((line, column)) = item.get("location").and_then(|l| start(l.get("range")))
            else {
                continue;
            };
            let container: Vec<String> = item
                .get("containerName")
                .and_then(Value::as_str)
                .map(|c| vec![c.to_string()])
                .unwrap_or_default();
            out.push(OutlineSymbol {
                name,
                detail: None,
                kind,
                line,
                column,
                depth: container.len(),
                container,
            });
        }
        out.sort_by_key(|s| (s.line, s.column));
        return out;
    }
    walk(array, &[], &mut out);
    out
}

fn walk(items: &[Value], container: &[String], out: &mut Vec<OutlineSymbol>) {
    let mut ordered: Vec<&Value> = items.iter().collect();
    ordered.sort_by_key(|item| start(item.get("range")).unwrap_or((0, 0)));
    for item in ordered {
        let (Some(name), Some(kind)) = (name_of(item), kind_of(item)) else {
            continue;
        };
        let Some((line, column)) =
            start(item.get("selectionRange")).or_else(|| start(item.get("range")))
        else {
            continue;
        };
        let detail = item
            .get("detail")
            .and_then(Value::as_str)
            .filter(|d| !d.is_empty())
            .map(String::from);
        out.push(OutlineSymbol {
            name: name.clone(),
            detail,
            kind,
            line,
            column,
            depth: container.len(),
            container: container.to_vec(),
        });
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            if !children.is_empty() {
                let mut inner = container.to_vec();
                inner.push(name);
                walk(children, &inner, out);
            }
        }
    }
}

/// `workspace/symbol` results (SymbolInformation or WorkspaceSymbol): each
/// symbol with the path of the file it's in. Symbols outside `file:` URIs
/// are left out.
pub fn parse_workspace(json: &[u8]) -> Vec<(OutlineSymbol, String)> {
    let Ok(Value::Array(array)) = serde_json::from_slice::<Value>(json) else {
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|item| {
            let name = name_of(item)?;
            let kind = kind_of(item)?;
            let location = item.get("location")?;
            let uri = location.get("uri")?.as_str()?;
            if Url::parse(uri).ok()?.scheme() != "file" {
                return None;
            }
            // WorkspaceSymbol may leave the range out until resolved: line 1.
            let (line, column) = start(location.get("range")).unwrap_or((1, 1));
            let container = item
                .get("containerName")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(|c| vec![c.to_string()])
                .unwrap_or_default();
            let symbol = OutlineSymbol {
                name,
                detail: None,
                kind,
                line,
                column,
                depth: 0,
                container,
            };
            Some((symbol, uri_to_file_path(uri)))
        })
        .collect()
}

fn name_of(item: &Value) -> Option<String> {
    item.get("name")?.as_str().map(String::from)
}

fn kind_of(item: &Value) -> Option<i64> {
    let kind = item.get("kind")?;
    kind.as_i64().or_else(|| kind.as_f64().map(|f| f as i64))
}

fn number(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value.as_i64().or_else(|| value.as_f64().map(|f| f as i64))
}

/// A range's start as 1-based (line, column).
fn start(range: Option<&Value>) -> Option<(usize, usize)> {
    let start = range?.get("start")?;
    let line = number(start.get("line"))?;
    let character = number(start.get("character"))?;
    Some(((line + 1).max(0) as usize, (character + 1).max(0) as usize))
}

pub fn kind_name(kind: i64) -> &'static str {
    const NAMES: [&str; 26] = [
        "file",
        "module",
        "namespace",
        "package",
        "class",
        "method",
        "property",
        "field",
        "constructor",
        "enum",
        "interface",
        "function",
        "variable",
        "constant",
        "string",
        "number",
        "boolean",
        "array",
        "object",
        "key",
        "null",
        "enum member",
        "struct",
        "event",
        "operator",
        "type parameter",
    ];
    usize::try_from(kind)
        .ok()
        .and_then(|k| k.checked_sub(1))
        .and_then(|i| NAMES.get(i))
        .copied()
        .unwrap_or("symbol")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchical_symbols_flatten_in_document_order() {
        let json = r#"
        [{"name": "Search", "kind": 23, "detail": "",
          "range": {"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 1}},
          "selectionRange": {"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 13}},
          "children": [
            {"name": "run", "kind": 6, "detail": "(String) -> [String]",
             "range": {"start": {"line": 5, "character": 2}, "end": {"line": 8, "character": 3}},
             "selectionRange": {"start": {"line": 5, "character": 7}, "end": {"line": 5, "character": 10}}},
            {"name": "index", "kind": 7,
             "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 20}},
             "selectionRange": {"start": {"line": 1, "character": 6}, "end": {"line": 1, "character": 11}}}]},
         {"name": "main", "kind": 12,
          "range": {"start": {"line": 11, "character": 0}, "end": {"line": 12, "character": 1}},
          "selectionRange": {"start": {"line": 11, "character": 5}, "end": {"line": 11, "character": 9}}}]
        "#;
        let symbols = parse(json.as_bytes());
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Search", "index", "run", "main"]);
        let depths: Vec<usize> = symbols.iter().map(|s| s.depth).collect();
        assert_eq!(depths, vec![0, 1, 1, 0]);
        assert_eq!(symbols[2].container, vec!["Search"]);
        // 1-based, from the selection range.
        assert_eq!((symbols[2].line, symbols[2].column), (6, 8));
        assert_eq!(symbols[2].detail.as_deref(), Some("(String) -> [String]"));
        assert_eq!(symbols[0].detail, None, "empty details are dropped");
        assert_eq!(symbols[0].kind_name(), "struct");
        assert_eq!(symbols[3].kind_name(), "function");
    }

    #[test]
    fn flat_symbol_information_uses_container_names() {
        let json = r#"
        [{"name": "helper", "kind": 12, "containerName": "Utils",
          "location": {"uri": "file:///a.py", "range": {"start": {"line": 4, "character": 4}, "end": {"line": 6, "character": 0}}}},
         {"name": "Utils", "kind": 5,
          "location": {"uri": "file:///a.py", "range": {"start": {"line": 2, "character": 0}, "end": {"line": 9, "character": 0}}}}]
        "#;
        let symbols = parse(json.as_bytes());
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Utils", "helper"]);
        assert_eq!(symbols[1].container, vec!["Utils"]);
        assert_eq!(symbols[1].depth, 1);
        assert!(parse(b"null").is_empty());
        assert_eq!(kind_name(99), "symbol");
        assert_eq!(kind_name(0), "symbol");
    }

    #[test]
    fn workspace_symbols_carry_their_files() {
        let json = r#"
        [{"name": "Search", "kind": 5, "containerName": "app",
          "location": {"uri": "file:///repo/src/search.ts", "range": {"start": {"line": 3, "character": 13}, "end": {"line": 3, "character": 19}}}},
         {"name": "lazy", "kind": 12, "location": {"uri": "file:///repo/a%20b.ts"}},
         {"name": "remote", "kind": 12, "location": {"uri": "https://example.com/x.ts"}}]
        "#;
        let results = parse_workspace(json.as_bytes());
        let paths: Vec<&str> = results.iter().map(|(_, p)| p.as_str()).collect();
        assert_eq!(paths, vec!["/repo/src/search.ts", "/repo/a b.ts"]);
        assert_eq!((results[0].0.line, results[0].0.column), (4, 14));
        assert_eq!(results[0].0.container, vec!["app"]);
        assert_eq!(results[1].0.line, 1, "no range yet");
    }
}
