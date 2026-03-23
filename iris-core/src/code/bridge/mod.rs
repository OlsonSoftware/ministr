//! Cross-language bridge detection framework.
//!
//! Bridges are the glue between languages in multi-language projects: Tauri
//! commands connecting Rust backends to TypeScript frontends, `napi-rs` exports
//! consumed from Node.js, `wasm-bindgen` functions called from JavaScript, etc.
//!
//! This module defines the core vocabulary:
//!
//! - [`BridgeKind`] — the mechanism category (Tauri, NAPI, `PyO3`, …)
//! - [`EndpointRole`] — whether a site exports or imports a binding
//! - [`BridgeEndpoint`] — one side of a bridge (a definition or a call site)
//! - [`BridgeLink`] — a matched export↔import pair
//! - [`BridgeExtractor`] — trait that concrete extractors implement
//! - [`ConfidenceLevel`] — standardized confidence scores for matching
//!
//! The [`linker`] submodule provides [`BridgeLinker`], a two-pass pipeline that
//! collects endpoints from all extractors and joins them by binding key.
//!
//! The [`detector`] submodule provides [`FrameworkDetector`] for auto-detecting
//! which bridge frameworks are present in a project.

pub mod detector;
pub mod linker;

use std::fmt;

use serde::{Deserialize, Serialize};

/// The kind of cross-language bridge mechanism.
///
/// Each variant represents a distinct interop technology with its own
/// annotation/invocation patterns. Concrete [`BridgeExtractor`] implementations
/// target exactly one kind.
///
/// # Examples
///
/// ```
/// use iris_core::code::bridge::BridgeKind;
///
/// let kind = BridgeKind::TauriCommand;
/// assert_eq!(kind.as_str(), "tauri_command");
/// assert_eq!(kind.to_string(), "tauri_command");
///
/// let parsed = BridgeKind::parse("wasm_bindgen");
/// assert_eq!(parsed, Some(BridgeKind::WasmBindgen));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeKind {
    /// Tauri `#[tauri::command]` ↔ `invoke("name")` IPC calls.
    TauriCommand,
    /// Tauri event system: `emit`/`listen` patterns across Rust and JS/TS.
    TauriEvent,
    /// `napi-rs` `#[napi]` exports consumed from JavaScript/TypeScript.
    Napi,
    /// `wasm-bindgen` `#[wasm_bindgen]` exports consumed from JavaScript.
    WasmBindgen,
    /// `PyO3` `#[pyfunction]`/`#[pyclass]`/`#[pymethods]` consumed from Python.
    PyO3,
    /// HTTP route annotations matched to client-side fetch/request calls.
    HttpRoute,
    /// Foreign function interface: `extern "C"`, ctypes, JNI, etc.
    Ffi,
}

impl BridgeKind {
    /// Returns the canonical string representation of this bridge kind.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TauriCommand => "tauri_command",
            Self::TauriEvent => "tauri_event",
            Self::Napi => "napi",
            Self::WasmBindgen => "wasm_bindgen",
            Self::PyO3 => "pyo3",
            Self::HttpRoute => "http_route",
            Self::Ffi => "ffi",
        }
    }

    /// Parse a bridge kind from its string representation.
    ///
    /// # Examples
    ///
    /// ```
    /// use iris_core::code::bridge::BridgeKind;
    ///
    /// assert_eq!(BridgeKind::parse("tauri_command"), Some(BridgeKind::TauriCommand));
    /// assert_eq!(BridgeKind::parse("napi"), Some(BridgeKind::Napi));
    /// assert_eq!(BridgeKind::parse("unknown"), None);
    /// ```
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tauri_command" => Some(Self::TauriCommand),
            "tauri_event" => Some(Self::TauriEvent),
            "napi" => Some(Self::Napi),
            "wasm_bindgen" => Some(Self::WasmBindgen),
            "pyo3" => Some(Self::PyO3),
            "http_route" => Some(Self::HttpRoute),
            "ffi" => Some(Self::Ffi),
            _ => None,
        }
    }
}

impl fmt::Display for BridgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a [`BridgeEndpoint`] defines/exports or consumes/imports a binding.
///
/// # Examples
///
/// ```
/// use iris_core::code::bridge::EndpointRole;
///
/// let role = EndpointRole::Export;
/// assert_eq!(role.as_str(), "export");
/// assert_eq!(role.to_string(), "export");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointRole {
    /// Defines or exposes the binding (e.g. `#[tauri::command] fn greet()`).
    Export,
    /// Consumes or calls the binding (e.g. `invoke("greet")`).
    Import,
}

impl EndpointRole {
    /// Returns the canonical string representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Export => "export",
            Self::Import => "import",
        }
    }
}

impl fmt::Display for EndpointRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Standardized confidence levels for bridge endpoint matching.
///
/// Each level corresponds to a fixed numeric score. Use these when
/// constructing [`BridgeEndpoint`]s to ensure consistent confidence
/// values across all extractors.
///
/// # Examples
///
/// ```
/// use iris_core::code::bridge::ConfidenceLevel;
///
/// assert!((ConfidenceLevel::Exact.score() - 1.0).abs() < f32::EPSILON);
/// assert!((ConfidenceLevel::CaseTransformed.score() - 0.9).abs() < f32::EPSILON);
/// assert!((ConfidenceLevel::Fuzzy.score() - 0.7).abs() < f32::EPSILON);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfidenceLevel {
    /// Exact string match between export and import binding keys.
    Exact,
    /// Cross-referenced with a handler registration macro (e.g.
    /// `tauri::generate_handler![]`), confirming the binding is valid.
    RegistrationValidated,
    /// Name matched after case transformation (e.g. `snake_case` → `camelCase`).
    CaseTransformed,
    /// Fuzzy or semantic match (e.g. embedding similarity, heuristic).
    Fuzzy,
}

impl ConfidenceLevel {
    /// Returns the numeric confidence score for this level.
    #[must_use]
    pub fn score(self) -> f32 {
        match self {
            Self::Exact | Self::RegistrationValidated => 1.0,
            Self::CaseTransformed => 0.9,
            Self::Fuzzy => 0.7,
        }
    }
}

impl fmt::Display for ConfidenceLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact => f.write_str("exact"),
            Self::RegistrationValidated => f.write_str("registration_validated"),
            Self::CaseTransformed => f.write_str("case_transformed"),
            Self::Fuzzy => f.write_str("fuzzy"),
        }
    }
}

/// One side of a cross-language bridge — an export or call site.
///
/// Endpoints are extracted per-file by [`BridgeExtractor`] implementations,
/// then joined by [`BridgeLinker`](linker::BridgeLinker) using `binding_key`
/// to form [`BridgeLink`]s.
///
/// # Examples
///
/// ```
/// use iris_core::code::bridge::{BridgeEndpoint, BridgeKind, EndpointRole};
///
/// let endpoint = BridgeEndpoint {
///     binding_key: "greet".into(),
///     kind: BridgeKind::TauriCommand,
///     role: EndpointRole::Export,
///     language: "rust".into(),
///     file_path: "src-tauri/src/main.rs".into(),
///     line: 42,
///     symbol_name: "greet".into(),
///     confidence: 1.0,
/// };
/// assert_eq!(endpoint.binding_key, "greet");
/// assert_eq!(endpoint.role, EndpointRole::Export);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeEndpoint {
    /// Canonical name used for joining endpoints across languages.
    ///
    /// This is the normalized binding key — e.g. `"greet"` for both
    /// `#[tauri::command] fn greet()` and `invoke("greet")`.
    pub binding_key: String,
    /// The bridge mechanism this endpoint belongs to.
    pub kind: BridgeKind,
    /// Whether this endpoint defines or consumes the binding.
    pub role: EndpointRole,
    /// The source language (e.g. `"rust"`, `"typescript"`).
    pub language: String,
    /// Source file path relative to the corpus root.
    pub file_path: String,
    /// Source line number where the endpoint appears.
    pub line: u32,
    /// The symbol name as it appears in source code.
    pub symbol_name: String,
    /// Confidence score in the range `0.0..=1.0`.
    ///
    /// - `1.0` — exact string match or registration-validated
    /// - `0.9` — case-transformed match (e.g. `snake_case` → `camelCase`)
    /// - `0.7` — fuzzy or semantic match
    pub confidence: f32,
}

/// A matched pair of endpoints forming a cross-language link.
///
/// Created by [`BridgeLinker`](linker::BridgeLinker) when an export and import
/// share the same `(BridgeKind, binding_key)`.
///
/// # Examples
///
/// ```
/// use iris_core::code::bridge::{BridgeEndpoint, BridgeKind, BridgeLink, EndpointRole};
///
/// let export = BridgeEndpoint {
///     binding_key: "greet".into(),
///     kind: BridgeKind::TauriCommand,
///     role: EndpointRole::Export,
///     language: "rust".into(),
///     file_path: "src-tauri/src/main.rs".into(),
///     line: 10,
///     symbol_name: "greet".into(),
///     confidence: 1.0,
/// };
/// let import = BridgeEndpoint {
///     binding_key: "greet".into(),
///     kind: BridgeKind::TauriCommand,
///     role: EndpointRole::Import,
///     language: "typescript".into(),
///     file_path: "src/App.tsx".into(),
///     line: 25,
///     symbol_name: "greet".into(),
///     confidence: 0.9,
/// };
/// let link = BridgeLink::new(export, import);
/// assert_eq!(link.kind, BridgeKind::TauriCommand);
/// assert!((link.confidence - 0.9).abs() < f32::EPSILON);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeLink {
    /// The export (definition) side of the bridge.
    pub export: BridgeEndpoint,
    /// The import (call site) side of the bridge.
    pub import: BridgeEndpoint,
    /// The bridge mechanism (always matches both endpoints).
    pub kind: BridgeKind,
    /// Combined confidence: `min(export.confidence, import.confidence)`.
    pub confidence: f32,
}

impl BridgeLink {
    /// Create a new bridge link from a matched export–import pair.
    ///
    /// The link's confidence is the minimum of both endpoint confidences,
    /// since the overall match is only as strong as its weakest side.
    ///
    /// # Panics
    ///
    /// Debug-asserts that both endpoints share the same `BridgeKind` and
    /// `binding_key`, and that roles are export/import respectively.
    #[must_use]
    pub fn new(export: BridgeEndpoint, import: BridgeEndpoint) -> Self {
        debug_assert_eq!(export.kind, import.kind, "endpoints must share BridgeKind");
        debug_assert_eq!(
            export.binding_key, import.binding_key,
            "endpoints must share binding_key"
        );
        debug_assert_eq!(export.role, EndpointRole::Export);
        debug_assert_eq!(import.role, EndpointRole::Import);

        let kind = export.kind;
        let confidence = export.confidence.min(import.confidence);
        Self {
            export,
            import,
            kind,
            confidence,
        }
    }
}

/// Trait for bridge-specific endpoint extractors.
///
/// Each implementation targets exactly one [`BridgeKind`] and knows how to
/// find export and import sites in the languages it supports. The
/// [`BridgeLinker`](linker::BridgeLinker) orchestrates multiple extractors
/// and joins their outputs.
///
/// # Implementors
///
/// Concrete implementations live in sibling modules (e.g. `tauri`, `napi`).
/// Each must declare:
///
/// - [`bridge_kind`](BridgeExtractor::bridge_kind) — which mechanism it handles
/// - [`applicable_languages`](BridgeExtractor::applicable_languages) — which
///   languages it can process
/// - [`extract_endpoints`](BridgeExtractor::extract_endpoints) — the actual
///   extraction logic using tree-sitter ASTs
pub trait BridgeExtractor: Send + Sync {
    /// The bridge mechanism this extractor targets.
    fn bridge_kind(&self) -> BridgeKind;

    /// Languages this extractor can process (canonical names).
    ///
    /// The linker only invokes this extractor for files whose detected
    /// language is in this list.
    fn applicable_languages(&self) -> &[&str];

    /// Extract bridge endpoints from a parsed tree-sitter AST.
    ///
    /// Returns all export and import sites found in the given source file.
    /// The `language` parameter is the canonical language name (e.g. `"rust"`,
    /// `"typescript"`) — guaranteed to be in [`applicable_languages`](Self::applicable_languages).
    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_kind_as_str_roundtrip() {
        let kinds = [
            BridgeKind::TauriCommand,
            BridgeKind::TauriEvent,
            BridgeKind::Napi,
            BridgeKind::WasmBindgen,
            BridgeKind::PyO3,
            BridgeKind::HttpRoute,
            BridgeKind::Ffi,
        ];
        for kind in kinds {
            let s = kind.as_str();
            let parsed = BridgeKind::parse(s);
            assert_eq!(parsed, Some(kind), "roundtrip failed for {s}");
        }
    }

    #[test]
    fn bridge_kind_parse_unknown() {
        assert_eq!(BridgeKind::parse("unknown"), None);
        assert_eq!(BridgeKind::parse(""), None);
    }

    #[test]
    fn bridge_kind_display() {
        assert_eq!(BridgeKind::TauriCommand.to_string(), "tauri_command");
        assert_eq!(BridgeKind::PyO3.to_string(), "pyo3");
    }

    #[test]
    fn bridge_kind_serde_roundtrip() {
        let kind = BridgeKind::WasmBindgen;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"wasm_bindgen\"");
        let back: BridgeKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn endpoint_role_as_str() {
        assert_eq!(EndpointRole::Export.as_str(), "export");
        assert_eq!(EndpointRole::Import.as_str(), "import");
    }

    #[test]
    fn endpoint_role_display() {
        assert_eq!(EndpointRole::Export.to_string(), "export");
        assert_eq!(EndpointRole::Import.to_string(), "import");
    }

    #[test]
    fn bridge_endpoint_construction() {
        let ep = BridgeEndpoint {
            binding_key: "greet".into(),
            kind: BridgeKind::TauriCommand,
            role: EndpointRole::Export,
            language: "rust".into(),
            file_path: "src-tauri/src/main.rs".into(),
            line: 42,
            symbol_name: "greet".into(),
            confidence: 1.0,
        };
        assert_eq!(ep.binding_key, "greet");
        assert_eq!(ep.kind, BridgeKind::TauriCommand);
        assert_eq!(ep.role, EndpointRole::Export);
        assert_eq!(ep.language, "rust");
    }

    #[test]
    fn bridge_endpoint_serde_roundtrip() {
        let ep = BridgeEndpoint {
            binding_key: "fetch_data".into(),
            kind: BridgeKind::Napi,
            role: EndpointRole::Import,
            language: "typescript".into(),
            file_path: "src/index.ts".into(),
            line: 10,
            symbol_name: "fetchData".into(),
            confidence: 0.9,
        };
        let json = serde_json::to_string(&ep).unwrap();
        let back: BridgeEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ep);
    }

    fn make_export(key: &str, kind: BridgeKind, confidence: f32) -> BridgeEndpoint {
        BridgeEndpoint {
            binding_key: key.into(),
            kind,
            role: EndpointRole::Export,
            language: "rust".into(),
            file_path: "src/lib.rs".into(),
            line: 1,
            symbol_name: key.into(),
            confidence,
        }
    }

    fn make_import(key: &str, kind: BridgeKind, confidence: f32) -> BridgeEndpoint {
        BridgeEndpoint {
            binding_key: key.into(),
            kind,
            role: EndpointRole::Import,
            language: "typescript".into(),
            file_path: "src/app.ts".into(),
            line: 5,
            symbol_name: key.into(),
            confidence,
        }
    }

    #[test]
    fn bridge_link_new_sets_min_confidence() {
        let export = make_export("greet", BridgeKind::TauriCommand, 1.0);
        let import = make_import("greet", BridgeKind::TauriCommand, 0.8);
        let link = BridgeLink::new(export, import);

        assert_eq!(link.kind, BridgeKind::TauriCommand);
        assert!((link.confidence - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn bridge_link_confidence_symmetric() {
        let export = make_export("cmd", BridgeKind::Napi, 0.7);
        let import = make_import("cmd", BridgeKind::Napi, 1.0);
        let link = BridgeLink::new(export, import);
        assert!((link.confidence - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn bridge_link_serde_roundtrip() {
        let export = make_export("run", BridgeKind::PyO3, 0.95);
        let import = make_import("run", BridgeKind::PyO3, 0.85);
        let link = BridgeLink::new(export, import);

        let json = serde_json::to_string(&link).unwrap();
        let back: BridgeLink = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, link.kind);
        assert!((back.confidence - link.confidence).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_level_scores() {
        assert!((ConfidenceLevel::Exact.score() - 1.0).abs() < f32::EPSILON);
        assert!((ConfidenceLevel::RegistrationValidated.score() - 1.0).abs() < f32::EPSILON);
        assert!((ConfidenceLevel::CaseTransformed.score() - 0.9).abs() < f32::EPSILON);
        assert!((ConfidenceLevel::Fuzzy.score() - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_level_display() {
        assert_eq!(ConfidenceLevel::Exact.to_string(), "exact");
        assert_eq!(
            ConfidenceLevel::RegistrationValidated.to_string(),
            "registration_validated"
        );
        assert_eq!(
            ConfidenceLevel::CaseTransformed.to_string(),
            "case_transformed"
        );
        assert_eq!(ConfidenceLevel::Fuzzy.to_string(), "fuzzy");
    }

    #[test]
    fn bridge_kind_ordering() {
        // BridgeKind should be Ord for use in BTreeSet
        let mut kinds = [BridgeKind::Ffi, BridgeKind::TauriCommand, BridgeKind::Napi];
        kinds.sort();
        // Just verify it doesn't panic and produces a deterministic order
        assert_eq!(kinds.len(), 3);
    }
}
