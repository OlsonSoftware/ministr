//! HTTP route bridge extractor for server-side route annotations and client-side API calls.
//!
//! Detects cross-language bridges between HTTP server routes and client fetch/request calls:
//!
//! - **Rust exports** — route macros from actix-web (`#[get("/path")]`, `#[post("/path")]`),
//!   Rocket (`#[get("/path")]`), and axum handler functions
//! - **Python exports** — Flask/FastAPI decorators (`@app.route("/path")`, `@app.get("/path")`)
//! - **JS/TS exports** — Express/Fastify route registrations (`app.get("/path", handler)`)
//! - **JS/TS imports** — `fetch("/path")`, `axios.get("/path")` client calls
//! - **Python imports** — `requests.get("/path")`, `httpx.get("/path")` client calls
//!
//! Implements [`BridgeExtractor`] and can be registered with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

// ---------------------------------------------------------------------------
// HttpRouteExtractor
// ---------------------------------------------------------------------------

/// Extracts HTTP route definitions and client API calls from source files.
///
/// **Server-side exports** (routes):
/// ```rust,ignore
/// #[get("/api/users")]
/// async fn get_users() -> impl Responder { /* ... */ }
///
/// #[post("/api/users")]
/// async fn create_user(body: Json<User>) -> impl Responder { /* ... */ }
/// ```
///
/// **Client-side imports** (API calls):
/// ```javascript,ignore
/// fetch("/api/users");
/// axios.get("/api/users");
/// axios.post("/api/users", data);
/// ```
///
/// The binding key is `"METHOD /path"` (e.g. `"GET /api/users"`).
/// The linker handles matching server routes to client calls.
pub struct HttpRouteExtractor;

impl BridgeExtractor for HttpRouteExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::HttpRoute
    }

    fn applicable_languages(&self) -> &[&str] {
        &["rust", "python", "javascript", "typescript"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        match language {
            "rust" => extract_rust_routes(tree, source, file_path),
            "python" => extract_python_endpoints(tree, source, file_path),
            "javascript" | "typescript" => extract_js_endpoints(tree, source, file_path, language),
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Binding key normalization
// ---------------------------------------------------------------------------

/// HTTP methods recognized by the extractor.
const HTTP_METHODS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

/// Normalize a route path: lowercase, strip trailing slash (except root "/").
fn normalize_path(path: &str) -> String {
    let path = path.trim().to_lowercase();
    if path.len() > 1 && path.ends_with('/') {
        path[..path.len() - 1].to_string()
    } else if path.is_empty() {
        "/".to_string()
    } else {
        path
    }
}

/// Build a canonical binding key from method + path.
///
/// Format: `"GET /api/users"` — uppercase method, normalized path.
fn make_binding_key(method: &str, path: &str) -> String {
    format!("{} {}", method.to_uppercase(), normalize_path(path))
}

// ---------------------------------------------------------------------------
// Rust route extraction (actix-web, Rocket)
// ---------------------------------------------------------------------------

/// Actix-web / Rocket HTTP method attribute names.
const RUST_ROUTE_ATTRS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "head", "options", "route",
];

/// Find route macro annotations on functions and produce Export endpoints.
fn extract_rust_routes(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_rust_routes(&mut cursor, source, file_path, &mut endpoints);
    endpoints
}

/// Recursively walk the tree looking for functions with HTTP route attributes.
fn walk_rust_routes(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();
        let kind = node.kind();

        if kind == "function_item" || kind == "function_definition" {
            if let Some((method, path)) = extract_route_attribute(&node, source) {
                let symbol_name =
                    rust_item_name(&node, source).unwrap_or_else(|| "<anonymous>".into());
                let binding_key = make_binding_key(&method, &path);
                #[allow(clippy::cast_possible_truncation)]
                let line = node.start_position().row as u32 + 1;
                endpoints.push(BridgeEndpoint {
                    binding_key,
                    kind: BridgeKind::HttpRoute,
                    role: EndpointRole::Export,
                    language: "rust".into(),
                    file_path: file_path.into(),
                    line,
                    symbol_name,
                    confidence: ConfidenceLevel::Exact.score(),
                });
            }
        }

        if cursor.goto_first_child() {
            walk_rust_routes(cursor, source, file_path, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Extract the HTTP method and path from a route attribute on the preceding siblings.
///
/// Handles: `#[get("/path")]`, `#[post("/path")]`, `#[route("/path", method = "GET")]`
fn extract_route_attribute(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
) -> Option<(String, String)> {
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() == "attribute_item" {
            let text = node_text(&sibling, source);
            if let Some(result) = parse_route_attribute_text(&text) {
                return Some(result);
            }
        } else if sibling.kind() != "attribute_item"
            && sibling.kind() != "line_comment"
            && sibling.kind() != "block_comment"
        {
            break;
        }
        prev = sibling.prev_sibling();
    }
    None
}

/// Parse a route attribute text like `#[get("/api/users")]` into (method, path).
fn parse_route_attribute_text(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    // Strip #[ and ]
    let inner = text.strip_prefix("#[")?.strip_suffix(']')?;

    // Check for method attributes: get("/path"), post("/path"), etc.
    for method in RUST_ROUTE_ATTRS {
        if method == &"route" {
            continue; // Handle #[route] separately
        }
        if let Some(rest) = inner.strip_prefix(method) {
            let rest = rest.trim();
            if let Some(path) = extract_first_string_literal(rest) {
                return Some(((*method).to_string(), path));
            }
        }
    }

    // Handle #[route("/path", method = "GET")]
    if let Some(rest) = inner.strip_prefix("route") {
        let rest = rest.trim();
        if let Some(path) = extract_first_string_literal(rest) {
            // Try to find method = "..."
            let method = extract_route_method_param(rest).unwrap_or_else(|| "GET".into());
            return Some((method.to_lowercase(), path));
        }
    }

    None
}

/// Extract the first string literal from parenthesized content: `("/foo")` → `"/foo"`.
fn extract_first_string_literal(s: &str) -> Option<String> {
    let s = s.trim();
    let inner = s.strip_prefix('(')?.strip_suffix(')')?;
    // Find first quoted string
    let start = inner.find('"')?;
    let rest = &inner[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Extract the method parameter from a `#[route(..., method = "GET")]` attribute.
fn extract_route_method_param(s: &str) -> Option<String> {
    // Look for `method = "GET"` or `method="POST"` pattern
    let lower = s.to_lowercase();
    let idx = lower.find("method")?;
    let rest = &s[idx..];
    // Find the = sign
    let eq_idx = rest.find('=')?;
    let after_eq = rest[eq_idx + 1..].trim();
    // Find the quoted method
    let start = after_eq.find('"')?;
    let rest2 = &after_eq[start + 1..];
    let end = rest2.find('"')?;
    Some(rest2[..end].to_string())
}

// ---------------------------------------------------------------------------
// Python endpoint extraction (Flask, FastAPI, Django + requests, httpx)
// ---------------------------------------------------------------------------

/// Extract Python HTTP endpoints: both server routes (exports) and client calls (imports).
fn extract_python_endpoints(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_python_endpoints(&mut cursor, source, file_path, &mut endpoints);
    endpoints
}

/// Python server-side decorator receiver names.
const PYTHON_ROUTE_RECEIVERS: &[&str] = &["app", "router", "blueprint", "bp"];

/// Python client library names.
const PYTHON_CLIENT_LIBS: &[&str] = &["requests", "httpx", "aiohttp", "client", "session"];

/// Recursively walk looking for Python route decorators and HTTP client calls.
fn walk_python_endpoints(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();

        match node.kind() {
            // Decorated functions: @app.get("/path") def handler():
            "decorated_definition" => {
                if let Some((method, path)) = extract_python_route_decorator(&node, source) {
                    let symbol_name = python_decorated_func_name(&node, source)
                        .unwrap_or_else(|| "<anonymous>".into());
                    let binding_key = make_binding_key(&method, &path);
                    #[allow(clippy::cast_possible_truncation)]
                    let line = node.start_position().row as u32 + 1;
                    endpoints.push(BridgeEndpoint {
                        binding_key,
                        kind: BridgeKind::HttpRoute,
                        role: EndpointRole::Export,
                        language: "python".into(),
                        file_path: file_path.into(),
                        line,
                        symbol_name,
                        confidence: ConfidenceLevel::Exact.score(),
                    });
                }
            }
            // Client calls: requests.get("/path"), httpx.post("/path")
            // Match at expression_statement level to avoid double-counting
            "expression_statement" => {
                if let Some((method, path)) = extract_python_client_call(&node, source) {
                    let binding_key = make_binding_key(&method, &path);
                    #[allow(clippy::cast_possible_truncation)]
                    let line = node.start_position().row as u32 + 1;
                    endpoints.push(BridgeEndpoint {
                        binding_key,
                        kind: BridgeKind::HttpRoute,
                        role: EndpointRole::Import,
                        language: "python".into(),
                        file_path: file_path.into(),
                        line,
                        symbol_name: format!("{method}(\"{path}\")"),
                        confidence: ConfidenceLevel::Exact.score(),
                    });
                }
            }
            _ => {}
        }

        if cursor.goto_first_child() {
            walk_python_endpoints(cursor, source, file_path, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Extract route method and path from a Python decorator like `@app.get("/path")`.
fn extract_python_route_decorator(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
) -> Option<(String, String)> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }

    loop {
        let child = cursor.node();
        if child.kind() == "decorator" {
            let text = node_text(&child, source);
            if let Some(result) = parse_python_decorator_text(&text) {
                return Some(result);
            }
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Parse decorator text like `@app.get("/path")` → ("get", "/path").
fn parse_python_decorator_text(text: &str) -> Option<(String, String)> {
    let text = text.strip_prefix('@')?;

    // Try "receiver.method("/path")" pattern
    let dot_idx = text.find('.')?;
    let receiver = &text[..dot_idx];
    let after_dot = &text[dot_idx + 1..];

    // Check if receiver is a known route receiver
    if !PYTHON_ROUTE_RECEIVERS.iter().any(|r| receiver.contains(r)) {
        return None;
    }

    // Extract method name and parenthesized content
    let paren_idx = after_dot.find('(')?;
    let method_name = after_dot[..paren_idx].trim();

    // Handle @app.route("/path", methods=["GET"]) and @app.get("/path")
    if method_name == "route" {
        let args = &after_dot[paren_idx..];
        let path = extract_first_string_literal_simple(args)?;
        // Try to find methods= parameter, default to GET
        let method = extract_python_route_methods_param(args).unwrap_or_else(|| "GET".into());
        return Some((method.to_lowercase(), path));
    }

    // Direct method: app.get, app.post, etc.
    if HTTP_METHODS.contains(&method_name.to_lowercase().as_str()) {
        let args = &after_dot[paren_idx..];
        let path = extract_first_string_literal_simple(args)?;
        return Some((method_name.to_lowercase(), path));
    }

    None
}

/// Extract first string literal from `("/foo", ...)`.
fn extract_first_string_literal_simple(s: &str) -> Option<String> {
    // Find first " or '
    let s = s.trim();
    for quote in ['"', '\''] {
        if let Some(start) = s.find(quote) {
            let rest = &s[start + 1..];
            if let Some(end) = rest.find(quote) {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Extract method from `methods=["GET"]` or `methods=("POST",)` in route decorator.
fn extract_python_route_methods_param(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    let idx = lower.find("methods")?;
    let rest = &s[idx..];
    // Find first string after methods=
    let eq_idx = rest.find('=')?;
    let after_eq = &rest[eq_idx + 1..];
    extract_first_string_literal_simple(after_eq)
}

/// Extract the function name from a decorated definition.
fn python_decorated_func_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "function_definition" {
            // Get the name field
            if let Some(name_node) = child.child_by_field_name("name") {
                return Some(node_text(&name_node, source));
            }
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Extract HTTP method and path from Python client calls like `requests.get("/path")`.
fn extract_python_client_call(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
) -> Option<(String, String)> {
    let text = node_text(node, source);
    for lib in PYTHON_CLIENT_LIBS {
        for method in HTTP_METHODS {
            let pattern = format!("{lib}.{method}(");
            if let Some(idx) = text.find(&pattern) {
                let after = &text[idx + pattern.len()..];
                if let Some(path) = extract_first_string_literal_simple(after) {
                    return Some(((*method).to_string(), path));
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// JS/TS endpoint extraction (Express, Fastify exports + fetch, axios imports)
// ---------------------------------------------------------------------------

/// Extract JS/TS HTTP endpoints: both server routes (exports) and client calls (imports).
fn extract_js_endpoints(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_js_endpoints(&mut cursor, source, file_path, language, &mut endpoints);
    endpoints
}

/// JS server-side route receiver patterns (Express/Fastify).
const JS_ROUTE_RECEIVERS: &[&str] = &["app", "router", "server", "fastify", "express"];

/// Recursively walk looking for JS/TS route registrations and client API calls.
fn walk_js_endpoints(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();

        if node.kind() == "expression_statement" {
            let text = node_text(&node, source);

            // Try server-side route: app.get("/path", handler)
            if let Some((method, path, role)) = parse_js_http_expression(&text) {
                let binding_key = make_binding_key(&method, &path);
                #[allow(clippy::cast_possible_truncation)]
                let line = node.start_position().row as u32 + 1;
                endpoints.push(BridgeEndpoint {
                    binding_key,
                    kind: BridgeKind::HttpRoute,
                    role,
                    language: language.into(),
                    file_path: file_path.into(),
                    line,
                    symbol_name: format!("{method}(\"{path}\")"),
                    confidence: ConfidenceLevel::Exact.score(),
                });
            }
        }
        // Also handle call expressions when they're top-level (e.g., inside await)
        else if node.kind() == "call_expression"
            && node
                .parent()
                .is_some_and(|p| p.kind() != "expression_statement")
        {
            let text = node_text(&node, source);

            if let Some((method, path, role)) = parse_js_http_expression(&text) {
                let binding_key = make_binding_key(&method, &path);
                #[allow(clippy::cast_possible_truncation)]
                let line = node.start_position().row as u32 + 1;
                endpoints.push(BridgeEndpoint {
                    binding_key,
                    kind: BridgeKind::HttpRoute,
                    role,
                    language: language.into(),
                    file_path: file_path.into(),
                    line,
                    symbol_name: format!("{method}(\"{path}\")"),
                    confidence: ConfidenceLevel::Exact.score(),
                });
            }
        }

        if cursor.goto_first_child() {
            walk_js_endpoints(cursor, source, file_path, language, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Parse a JS expression to detect HTTP route registrations or client calls.
///
/// Returns `(method, path, role)`.
fn parse_js_http_expression(text: &str) -> Option<(String, String, EndpointRole)> {
    let text = text.trim().trim_end_matches(';');

    // Server-side: app.get("/path", handler), router.post("/path", handler)
    for receiver in JS_ROUTE_RECEIVERS {
        for method in HTTP_METHODS {
            let pattern = format!("{receiver}.{method}(");
            if let Some(idx) = text.find(&pattern) {
                let after = &text[idx + pattern.len()..];
                if let Some(path) = extract_first_string_literal_simple(after) {
                    return Some((method.to_string(), path, EndpointRole::Export));
                }
            }
        }
    }

    // Client-side: fetch("/path") or fetch("/path", { method: "POST" })
    if let Some(idx) = text.find("fetch(") {
        let after = &text[idx + 6..];
        if let Some(path) = extract_first_string_literal_simple(after) {
            // Check for method in options
            let method = extract_fetch_method(after).unwrap_or_else(|| "GET".into());
            return Some((method.to_lowercase(), path, EndpointRole::Import));
        }
    }

    // Client-side: axios.get("/path"), axios.post("/path", data)
    for method in HTTP_METHODS {
        let pattern = format!("axios.{method}(");
        if let Some(idx) = text.find(&pattern) {
            let after = &text[idx + pattern.len()..];
            if let Some(path) = extract_first_string_literal_simple(after) {
                return Some((method.to_string(), path, EndpointRole::Import));
            }
        }
    }

    None
}

/// Extract the HTTP method from a `fetch()` options object.
///
/// Handles: `fetch("/path", { method: "POST" })`
fn extract_fetch_method(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    let idx = lower.find("method")?;
    let rest = &s[idx..];
    // Find the next quoted string after "method"
    extract_first_string_literal_simple(rest)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Extract UTF-8 text from a tree-sitter node.
fn node_text(node: &tree_sitter::Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

/// Extract the name identifier from a Rust function item.
fn rust_item_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "identifier" && cursor.field_name() == Some("name") {
            return Some(node_text(&child, source));
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    #[cfg(feature = "lang-python")]
    fn parse_python(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    #[cfg(feature = "lang-javascript")]
    fn parse_js(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    #[cfg(feature = "lang-typescript")]
    fn parse_ts(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    // -- Binding key normalization --

    #[test]
    fn normalize_path_strips_trailing_slash() {
        assert_eq!(normalize_path("/api/users/"), "/api/users");
        assert_eq!(normalize_path("/api/users"), "/api/users");
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn normalize_path_lowercases() {
        assert_eq!(normalize_path("/API/Users"), "/api/users");
    }

    #[test]
    fn make_binding_key_format() {
        assert_eq!(make_binding_key("get", "/api/users"), "GET /api/users");
        assert_eq!(make_binding_key("POST", "/api/items/"), "POST /api/items");
    }

    // -- Attribute text parsing --

    #[test]
    fn parse_route_attr_get() {
        let result = parse_route_attribute_text("#[get(\"/api/users\")]");
        assert_eq!(result, Some(("get".into(), "/api/users".into())));
    }

    #[test]
    fn parse_route_attr_post() {
        let result = parse_route_attribute_text("#[post(\"/api/items\")]");
        assert_eq!(result, Some(("post".into(), "/api/items".into())));
    }

    #[test]
    fn parse_route_attr_route_with_method() {
        let result = parse_route_attribute_text("#[route(\"/api/data\", method = \"PUT\")]");
        assert_eq!(result, Some(("put".into(), "/api/data".into())));
    }

    #[test]
    fn parse_route_attr_non_route_returns_none() {
        assert_eq!(parse_route_attribute_text("#[derive(Debug)]"), None);
        assert_eq!(parse_route_attribute_text("#[test]"), None);
    }

    // -- Rust route extraction --

    #[test]
    fn rust_actix_get_route() {
        let source = r#"
#[get("/api/users")]
async fn get_users() -> impl Responder {
    HttpResponse::Ok().json(vec!["alice", "bob"])
}
"#;
        let tree = parse_rust(source);
        let extractor = HttpRouteExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/main.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "GET /api/users");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
        assert_eq!(endpoints[0].kind, BridgeKind::HttpRoute);
        assert_eq!(endpoints[0].symbol_name, "get_users");
        assert!((endpoints[0].confidence - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn rust_actix_post_route() {
        let source = r#"
#[post("/api/items")]
async fn create_item(body: Json<Item>) -> impl Responder {
    HttpResponse::Created()
}
"#;
        let tree = parse_rust(source);
        let extractor = HttpRouteExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/main.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "POST /api/items");
        assert_eq!(endpoints[0].symbol_name, "create_item");
    }

    #[test]
    fn rust_multiple_routes() {
        let source = r#"
#[get("/api/users")]
async fn list_users() -> impl Responder {
    HttpResponse::Ok()
}

#[post("/api/users")]
async fn create_user() -> impl Responder {
    HttpResponse::Created()
}

#[delete("/api/users/{id}")]
async fn delete_user() -> impl Responder {
    HttpResponse::NoContent()
}
"#;
        let tree = parse_rust(source);
        let extractor = HttpRouteExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/main.rs", "rust");

        assert_eq!(endpoints.len(), 3);
        let keys: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(keys.contains(&"GET /api/users"));
        assert!(keys.contains(&"POST /api/users"));
        assert!(keys.contains(&"DELETE /api/users/{id}"));
    }

    #[test]
    fn rust_no_route_attribute() {
        let source = r#"
#[derive(Debug)]
struct Foo;

fn regular_function() -> String { "hello".into() }
"#;
        let tree = parse_rust(source);
        let extractor = HttpRouteExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/main.rs", "rust");

        assert!(endpoints.is_empty());
    }

    // -- Python route extraction --

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_flask_route_decorator() {
        let source = r#"
@app.get("/api/users")
def get_users():
    return jsonify(users)
"#;
        let tree = parse_python(source);
        let extractor = HttpRouteExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "app.py", "python");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "GET /api/users");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
        assert_eq!(endpoints[0].symbol_name, "get_users");
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_fastapi_post_route() {
        let source = r#"
@app.post("/api/items")
def create_item(item: Item):
    return {"id": 1, **item.dict()}
"#;
        let tree = parse_python(source);
        let extractor = HttpRouteExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "app.py", "python");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "POST /api/items");
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_route_with_methods() {
        let source = r#"
@app.route("/api/data", methods=["PUT"])
def update_data():
    return "ok"
"#;
        let tree = parse_python(source);
        let extractor = HttpRouteExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "app.py", "python");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "PUT /api/data");
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_requests_client_call() {
        let source = r#"
response = requests.get("/api/users")
data = requests.post("/api/items", json=payload)
"#;
        let tree = parse_python(source);
        let extractor = HttpRouteExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "client.py", "python");

        assert_eq!(endpoints.len(), 2);
        let keys: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(keys.contains(&"GET /api/users"));
        assert!(keys.contains(&"POST /api/items"));
        for ep in &endpoints {
            assert_eq!(ep.role, EndpointRole::Import);
        }
    }

    // -- JS/TS route extraction --

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_express_route() {
        let source = r#"
app.get("/api/users", (req, res) => {
    res.json(users);
});

app.post("/api/users", (req, res) => {
    res.status(201).json(req.body);
});
"#;
        let tree = parse_js(source);
        let extractor = HttpRouteExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "server.js", "javascript");

        assert_eq!(endpoints.len(), 2);
        let keys: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(keys.contains(&"GET /api/users"));
        assert!(keys.contains(&"POST /api/users"));
        for ep in &endpoints {
            assert_eq!(ep.role, EndpointRole::Export);
        }
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_fetch_client_call() {
        let source = r#"
fetch("/api/users");
fetch("/api/items", { method: "POST", body: JSON.stringify(data) });
"#;
        let tree = parse_js(source);
        let extractor = HttpRouteExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "client.js", "javascript");

        assert_eq!(endpoints.len(), 2);
        let keys: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(keys.contains(&"GET /api/users"));
        assert!(keys.contains(&"POST /api/items"));
        for ep in &endpoints {
            assert_eq!(ep.role, EndpointRole::Import);
        }
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_axios_client_call() {
        let source = r#"
axios.get("/api/users");
axios.post("/api/items", data);
"#;
        let tree = parse_js(source);
        let extractor = HttpRouteExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "client.js", "javascript");

        assert_eq!(endpoints.len(), 2);
        let keys: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(keys.contains(&"GET /api/users"));
        assert!(keys.contains(&"POST /api/items"));
        for ep in &endpoints {
            assert_eq!(ep.role, EndpointRole::Import);
        }
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn ts_fetch_client_call() {
        let source = r#"
const users = await fetch("/api/users");
const result = await fetch("/api/items", { method: "POST", body: JSON.stringify(data) });
"#;
        let tree = parse_ts(source);
        let extractor = HttpRouteExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "client.ts", "typescript");

        assert_eq!(endpoints.len(), 2);
    }

    // -- Integration: Rust server ↔ JS client --

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn http_route_full_link_rust_to_js() {
        use super::super::linker::{BridgeLinker, SourceFile};

        let rust_source = r#"
#[get("/api/users")]
async fn get_users() -> impl Responder {
    HttpResponse::Ok()
}
"#;
        let js_source = r#"
fetch("/api/users");
"#;
        let rust_tree = parse_rust(rust_source);
        let js_tree = parse_js(js_source);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(HttpRouteExtractor));

        let files = [
            SourceFile {
                file_path: "src/main.rs",
                language: "rust",
                tree: &rust_tree,
                source: rust_source.as_bytes(),
            },
            SourceFile {
                file_path: "src/client.js",
                language: "javascript",
                tree: &js_tree,
                source: js_source.as_bytes(),
            },
        ];

        let links = linker.extract_and_link(&files);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, BridgeKind::HttpRoute);
        assert_eq!(links[0].export.binding_key, "GET /api/users");
        assert_eq!(links[0].export.language, "rust");
        assert_eq!(links[0].import.language, "javascript");
    }

    // -- Integration: Python server ↔ JS client --

    #[cfg(all(feature = "lang-python", feature = "lang-javascript"))]
    #[test]
    fn http_route_full_link_python_to_js() {
        use super::super::linker::{BridgeLinker, SourceFile};

        let python_source = r#"
@app.get("/api/items")
def list_items():
    return items
"#;
        let js_source = r#"
const items = await fetch("/api/items");
"#;
        let python_tree = parse_python(python_source);
        let js_tree = parse_js(js_source);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(HttpRouteExtractor));

        let files = [
            SourceFile {
                file_path: "app.py",
                language: "python",
                tree: &python_tree,
                source: python_source.as_bytes(),
            },
            SourceFile {
                file_path: "client.js",
                language: "javascript",
                tree: &js_tree,
                source: js_source.as_bytes(),
            },
        ];

        let links = linker.extract_and_link(&files);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, BridgeKind::HttpRoute);
        assert_eq!(links[0].export.language, "python");
        assert_eq!(links[0].import.language, "javascript");
    }
}
