//! Electron IPC bridge extractor — main ↔ renderer process channels.
//!
//! - **main-process exports** — `ipcMain.handle('chan', …)` /
//!   `ipcMain.on('chan', …)` / `ipcMain.once('chan', …)`. Binding key =
//!   the channel-name string literal.
//! - **renderer imports** — `ipcRenderer.invoke('chan')` /
//!   `ipcRenderer.send('chan')` / `ipcRenderer.sendSync('chan')` /
//!   `ipcRenderer.on('chan', …)`.
//!
//! Channel-name string literals are distinctive and name-only matching
//! has a low false-positive rate; unmatched endpoints never link.
//!
//! Implements [`BridgeExtractor`]; register with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::util::{node_line, node_text};
use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

const MAIN_METHODS: &[&str] = &["handle", "handleOnce", "on", "once"];
const RENDERER_METHODS: &[&str] = &["invoke", "send", "sendSync", "sendTo", "on", "once"];

/// Extracts Electron IPC channel bindings.
pub struct ElectronIpcExtractor;

impl BridgeExtractor for ElectronIpcExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::ElectronIpc
    }

    fn applicable_languages(&self) -> &[&str] {
        &["javascript", "typescript", "tsx"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        let mut endpoints = Vec::new();
        let mut cursor = tree.walk();
        walk(&mut cursor, &mut |node| {
            if node.kind() != "call_expression" {
                return;
            }
            let Some(func) = node.child_by_field_name("function") else {
                return;
            };
            if func.kind() != "member_expression" {
                return;
            }
            let Some(obj) = func.child_by_field_name("object") else {
                return;
            };
            let Some(prop) = func.child_by_field_name("property") else {
                return;
            };
            let obj_name = node_text(&obj, source);
            let method = node_text(&prop, source);

            let role = match obj_name.as_str() {
                "ipcMain" if MAIN_METHODS.contains(&method.as_str()) => EndpointRole::Export,
                "ipcRenderer" if RENDERER_METHODS.contains(&method.as_str()) => {
                    EndpointRole::Import
                }
                _ => return,
            };

            let Some(args) = node.child_by_field_name("arguments") else {
                return;
            };
            let mut c = args.walk();
            if let Some(first) = args
                .children(&mut c)
                .find(|n| matches!(n.kind(), "string" | "template_string"))
            {
                let raw = node_text(&first, source);
                let chan = raw.trim_matches(['"', '\'', '`']).trim();
                if !chan.is_empty() {
                    endpoints.push(BridgeEndpoint {
                        binding_key: chan.to_string(),
                        kind: BridgeKind::ElectronIpc,
                        role,
                        language: language.into(),
                        file_path: file_path.into(),
                        line: node_line(node),
                        symbol_name: format!("{obj_name}.{method}"),
                        confidence: ConfidenceLevel::Exact.score(),
                    });
                }
            }
        });
        endpoints
    }
}

fn walk(cursor: &mut tree_sitter::TreeCursor<'_>, visit: &mut dyn FnMut(&tree_sitter::Node<'_>)) {
    loop {
        visit(&cursor.node());
        if cursor.goto_first_child() {
            walk(cursor, visit);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::GrammarRegistry;

    fn parse(lang: &str, src: &str) -> tree_sitter::Tree {
        let l = GrammarRegistry::global()
            .language_by_name(lang)
            .expect("registered");
        let mut p = tree_sitter::Parser::new();
        p.set_language(l).unwrap();
        p.parse(src, None).unwrap()
    }

    #[test]
    fn electron_main_to_renderer_link() {
        use super::super::linker::{BridgeLinker, SourceFile};
        let main = "ipcMain.handle('get-config', async () => ({}));\n";
        let renderer = "const c = await ipcRenderer.invoke('get-config');\n";
        let mt = parse("javascript", main);
        let rt = parse("javascript", renderer);
        let mut linker = BridgeLinker::new();
        linker.register(Box::new(ElectronIpcExtractor));
        let files = [
            SourceFile {
                file_path: "main.js",
                language: "javascript",
                tree: &mt,
                source: main.as_bytes(),
            },
            SourceFile {
                file_path: "renderer.js",
                language: "javascript",
                tree: &rt,
                source: renderer.as_bytes(),
            },
        ];
        let links = linker.extract_and_link(&files);
        assert!(
            links.iter().any(|l| l.kind == BridgeKind::ElectronIpc
                && l.export.binding_key == "get-config"),
            "links: {links:?}"
        );
    }
}
