//! Grammar registry mapping file extensions to tree-sitter languages.
//!
//! The [`GrammarRegistry`] provides on-demand access to tree-sitter language
//! grammars. Grammars are compiled in at build time via cargo feature flags
//! (e.g. `lang-python`, `lang-typescript`). The registry maps file extensions
//! to language names and resolves the appropriate [`tree_sitter::Language`].

use std::collections::HashMap;
use std::sync::OnceLock;

/// Metadata for a supported language grammar.
#[derive(Debug, Clone)]
pub struct LanguageGrammar {
    /// Canonical language name (e.g. "rust", "python", "typescript").
    pub name: &'static str,
    /// File extensions that map to this language (without leading dot).
    pub extensions: &'static [&'static str],
}

/// A registry of available tree-sitter language grammars.
///
/// The registry is populated at build time based on enabled cargo features.
/// It maps file extensions to [`tree_sitter::Language`] instances and provides
/// lookup by extension or language name.
///
/// # Examples
///
/// ```
/// use ministr_core::code::GrammarRegistry;
///
/// let registry = GrammarRegistry::global();
/// // Rust is always available (not feature-gated)
/// let lang = registry.language_for_extension("rs");
/// assert!(lang.is_some());
/// ```
pub struct GrammarRegistry {
    /// Extension → language name.
    ext_to_lang: HashMap<&'static str, &'static str>,
    /// Language name → `tree_sitter::Language`.
    languages: HashMap<&'static str, tree_sitter::Language>,
    /// Language name → grammar metadata.
    grammars: HashMap<&'static str, LanguageGrammar>,
}

/// Global singleton registry.
static GLOBAL_REGISTRY: OnceLock<GrammarRegistry> = OnceLock::new();

impl GrammarRegistry {
    /// Get the global grammar registry singleton.
    ///
    /// The registry is lazily initialized on first access.
    #[must_use]
    pub fn global() -> &'static Self {
        GLOBAL_REGISTRY.get_or_init(Self::build)
    }

    /// Build the registry from compiled-in grammars.
    #[allow(clippy::too_many_lines)]
    fn build() -> Self {
        let mut ext_to_lang = HashMap::new();
        let mut languages = HashMap::new();
        let mut grammars = HashMap::new();

        // Rust — always available (not feature-gated)
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "rust",
            &["rs"],
            tree_sitter_rust::LANGUAGE.into(),
        );

        // Python
        #[cfg(feature = "lang-python")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "python",
            &["py", "pyi", "pyw"],
            tree_sitter_python::LANGUAGE.into(),
        );

        // JavaScript
        #[cfg(feature = "lang-javascript")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "javascript",
            &["js", "mjs", "cjs", "jsx"],
            tree_sitter_javascript::LANGUAGE.into(),
        );

        // TypeScript
        #[cfg(feature = "lang-typescript")]
        {
            register_language(
                &mut ext_to_lang,
                &mut languages,
                &mut grammars,
                "typescript",
                &["ts", "mts", "cts"],
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            );
            register_language(
                &mut ext_to_lang,
                &mut languages,
                &mut grammars,
                "tsx",
                &["tsx"],
                tree_sitter_typescript::LANGUAGE_TSX.into(),
            );
        }

        // Go
        #[cfg(feature = "lang-go")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "go",
            &["go"],
            tree_sitter_go::LANGUAGE.into(),
        );

        // Java
        #[cfg(feature = "lang-java")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "java",
            &["java"],
            tree_sitter_java::LANGUAGE.into(),
        );

        // C
        #[cfg(feature = "lang-c")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "c",
            &["c", "h"],
            tree_sitter_c::LANGUAGE.into(),
        );

        // C++ — using the Unreal-aware grammar (strict superset of
        // tree-sitter-cpp). Vanilla C++ parses identically; UE
        // reflection macros (UCLASS / UFUNCTION / GENERATED_BODY /
        // ...) get recognized as first-class nodes instead of
        // exploding into ERROR subtrees.
        #[cfg(feature = "lang-cpp")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "cpp",
            &["cpp", "cc", "cxx", "hpp", "hxx", "hh"],
            tree_sitter_unreal_cpp::LANGUAGE.into(),
        );

        // Ruby
        #[cfg(feature = "lang-ruby")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "ruby",
            &["rb", "rake", "gemspec"],
            tree_sitter_ruby::LANGUAGE.into(),
        );

        // C#
        #[cfg(feature = "lang-csharp")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "csharp",
            &["cs"],
            tree_sitter_c_sharp::LANGUAGE.into(),
        );

        // Swift
        #[cfg(feature = "lang-swift")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "swift",
            &["swift"],
            tree_sitter_swift::LANGUAGE.into(),
        );

        // Kotlin
        #[cfg(feature = "lang-kotlin")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "kotlin",
            &["kt", "kts"],
            tree_sitter_kotlin_ng::LANGUAGE.into(),
        );

        // Bash / Shell
        #[cfg(feature = "lang-bash")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "bash",
            &["sh", "bash", "zsh"],
            tree_sitter_bash::LANGUAGE.into(),
        );

        // PHP
        #[cfg(feature = "lang-php")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "php",
            &["php"],
            tree_sitter_php::LANGUAGE_PHP.into(),
        );

        // Scala
        #[cfg(feature = "lang-scala")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "scala",
            &["scala", "sc"],
            tree_sitter_scala::LANGUAGE.into(),
        );

        // Lua
        #[cfg(feature = "lang-lua")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "lua",
            &["lua"],
            tree_sitter_lua::LANGUAGE.into(),
        );

        // Elixir
        #[cfg(feature = "lang-elixir")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "elixir",
            &["ex", "exs"],
            tree_sitter_elixir::LANGUAGE.into(),
        );

        // Haskell
        #[cfg(feature = "lang-haskell")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "haskell",
            &["hs"],
            tree_sitter_haskell::LANGUAGE.into(),
        );

        // OCaml — separate grammars for implementations (.ml) and
        // interfaces (.mli).
        #[cfg(feature = "lang-ocaml")]
        {
            register_language(
                &mut ext_to_lang,
                &mut languages,
                &mut grammars,
                "ocaml",
                &["ml"],
                tree_sitter_ocaml::LANGUAGE_OCAML.into(),
            );
            register_language(
                &mut ext_to_lang,
                &mut languages,
                &mut grammars,
                "ocaml_interface",
                &["mli"],
                tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
            );
        }

        // Dart
        #[cfg(feature = "lang-dart")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "dart",
            &["dart"],
            tree_sitter_dart::language(),
        );

        // R
        #[cfg(feature = "lang-r")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "r",
            &["r", "R"],
            tree_sitter_r::LANGUAGE.into(),
        );

        // HCL / Terraform
        #[cfg(feature = "lang-hcl")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "hcl",
            // `.tfvars` is plain HCL (Terraform variable definitions) and
            // is ubiquitous in real Terraform repos — parse it like `.tf`.
            &["tf", "tfvars", "hcl"],
            tree_sitter_hcl::LANGUAGE.into(),
        );

        // JSON
        #[cfg(feature = "lang-json")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "json",
            &["json", "jsonc"],
            tree_sitter_json::LANGUAGE.into(),
        );

        // YAML
        #[cfg(feature = "lang-yaml")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "yaml",
            &["yml", "yaml"],
            tree_sitter_yaml::LANGUAGE.into(),
        );

        // TOML
        #[cfg(feature = "lang-toml")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "toml",
            &["toml"],
            tree_sitter_toml_ng::LANGUAGE.into(),
        );

        // SQL
        #[cfg(feature = "lang-sql")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "sql",
            &["sql"],
            tree_sitter_sequel::LANGUAGE.into(),
        );

        // Zig
        #[cfg(feature = "lang-zig")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "zig",
            &["zig"],
            tree_sitter_zig::LANGUAGE.into(),
        );

        // Protobuf
        #[cfg(feature = "lang-proto")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "proto",
            &["proto"],
            tree_sitter_proto::LANGUAGE.into(),
        );

        // Svelte — a composite single-file component grammar (markup +
        // embedded <script>/<style>). The host grammar models the SFC
        // structure; deep JS/CSS injection is a follow-up.
        #[cfg(feature = "lang-svelte")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "svelte",
            &["svelte"],
            tree_sitter_svelte_ng::LANGUAGE.into(),
        );

        // --- Maximal coverage expansion -------------------------------
        // All use the tree-sitter-language 0.1 ABI shim (tree-sitter
        // only a dev-dep upstream), so they link cleanly against our
        // 0.26 core. Clojure/Dockerfile/Vue/Astro lack an ABI-current
        // grammar and keep the text fallback (see root Cargo.toml).

        // CSS / SCSS
        #[cfg(feature = "lang-css")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "css",
            &["css", "scss"],
            tree_sitter_css::LANGUAGE.into(),
        );

        // (Markdown and HTML intentionally NOT registered here:
        // `detect_parser_kind` routes .md/.markdown → ParserKind::Markdown
        // and .html/.htm → ParserKind::Html *before* the code path, so
        // their dedicated prose/markup parsers — better than a code AST
        // for those formats — always take precedence. A code grammar
        // here would be unreachable dead weight.)

        // GraphQL
        #[cfg(feature = "lang-graphql")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "graphql",
            &["graphql", "gql"],
            tree_sitter_graphql::LANGUAGE.into(),
        );

        // Groovy / Gradle
        #[cfg(feature = "lang-groovy")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "groovy",
            &["groovy", "gradle"],
            tree_sitter_groovy::LANGUAGE.into(),
        );

        // Nix
        #[cfg(feature = "lang-nix")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "nix",
            &["nix"],
            tree_sitter_nix::LANGUAGE.into(),
        );

        // Erlang
        #[cfg(feature = "lang-erlang")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "erlang",
            &["erl", "hrl"],
            tree_sitter_erlang::LANGUAGE.into(),
        );

        // PowerShell
        #[cfg(feature = "lang-powershell")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "powershell",
            &["ps1", "psm1", "psd1"],
            tree_sitter_powershell::LANGUAGE.into(),
        );

        // Solidity
        #[cfg(feature = "lang-solidity")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "solidity",
            &["sol"],
            tree_sitter_solidity::LANGUAGE.into(),
        );

        // Objective-C
        #[cfg(feature = "lang-objc")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "objc",
            &["m", "mm"],
            tree_sitter_objc::LANGUAGE.into(),
        );

        // Julia
        #[cfg(feature = "lang-julia")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "julia",
            &["jl"],
            tree_sitter_julia::LANGUAGE.into(),
        );

        // CMake
        #[cfg(feature = "lang-cmake")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "cmake",
            &["cmake"],
            tree_sitter_cmake::LANGUAGE.into(),
        );

        // Make
        #[cfg(feature = "lang-make")]
        register_language(
            &mut ext_to_lang,
            &mut languages,
            &mut grammars,
            "make",
            &["mk", "make"],
            tree_sitter_make::LANGUAGE.into(),
        );

        Self {
            ext_to_lang,
            languages,
            grammars,
        }
    }

    /// Look up a tree-sitter language by file extension (without leading dot).
    ///
    /// Returns `None` if the extension is not registered or the grammar
    /// feature is not enabled.
    #[must_use]
    pub fn language_for_extension(&self, ext: &str) -> Option<&tree_sitter::Language> {
        let lang_name = self.ext_to_lang.get(ext)?;
        self.languages.get(lang_name)
    }

    /// Look up a tree-sitter language by canonical name (e.g. "rust", "python").
    #[must_use]
    pub fn language_by_name(&self, name: &str) -> Option<&tree_sitter::Language> {
        self.languages.get(name)
    }

    /// Get the canonical language name for a file extension.
    #[must_use]
    pub fn language_name_for_extension(&self, ext: &str) -> Option<&'static str> {
        self.ext_to_lang.get(ext).copied()
    }

    /// Get grammar metadata for a language.
    #[must_use]
    pub fn grammar(&self, name: &str) -> Option<&LanguageGrammar> {
        self.grammars.get(name)
    }

    /// All registered file extensions.
    pub fn extensions(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.ext_to_lang.keys().copied()
    }

    /// All registered language names.
    pub fn language_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.languages.keys().copied()
    }

    /// Whether a file extension has an available grammar.
    #[must_use]
    pub fn supports_extension(&self, ext: &str) -> bool {
        self.ext_to_lang.contains_key(ext)
    }

    /// Number of registered languages.
    #[must_use]
    pub fn language_count(&self) -> usize {
        self.languages.len()
    }
}

/// Helper to register a language in all three maps.
fn register_language(
    ext_to_lang: &mut HashMap<&'static str, &'static str>,
    languages: &mut HashMap<&'static str, tree_sitter::Language>,
    grammars: &mut HashMap<&'static str, LanguageGrammar>,
    name: &'static str,
    extensions: &'static [&'static str],
    language: tree_sitter::Language,
) {
    for ext in extensions {
        ext_to_lang.insert(ext, name);
    }
    languages.insert(name, language);
    grammars.insert(name, LanguageGrammar { name, extensions });
}

/// All file extensions known to the grammar registry, including those for
/// grammars not currently compiled in. Used by [`detect_parser_kind`] to
/// route files to the Code parser even when the specific grammar is absent
/// (falls back to text-based heuristics).
///
/// [`detect_parser_kind`]: crate::parser::detect_parser_kind
pub const ALL_CODE_EXTENSIONS: &[&str] = &[
    // Rust (always available)
    "rs",
    // Python
    "py",
    "pyi",
    "pyw",
    // JavaScript
    "js",
    "mjs",
    "cjs",
    "jsx",
    // TypeScript
    "ts",
    "mts",
    "cts",
    "tsx",
    // Go
    "go",
    // Java
    "java",
    // C
    "c",
    "h",
    // C++
    "cpp",
    "cc",
    "cxx",
    "hpp",
    "hxx",
    "hh",
    // C#
    "cs",
    // Ruby
    "rb",
    "rake",
    "gemspec",
    // Swift
    "swift",
    // Kotlin
    "kt",
    "kts",
    // Scala
    "scala",
    "sc",
    // PHP
    "php",
    // Elixir
    "ex",
    "exs",
    // Haskell
    "hs",
    // Lua
    "lua",
    // Zig
    "zig",
    // OCaml
    "ml",
    "mli",
    // Dart
    "dart",
    // R
    "r",
    "R",
    // Shell / Bash
    "sh",
    "bash",
    "zsh",
    // SQL
    "sql",
    // YAML
    "yml",
    "yaml",
    // TOML
    "toml",
    // JSON
    "json",
    "jsonc",
    // HCL / Terraform
    "tf",
    "tfvars",
    "hcl",
    // Dockerfile
    "dockerfile",
    // Protobuf
    "proto",
    // Composite / single-file components. `.svelte` has a registered
    // grammar; `.vue`/`.astro` route here for text-level indexing until
    // ABI-current grammars exist.
    "svelte",
    "vue",
    "astro",
    // Assembly
    "asm",
    "s",
    "S",
    "inc",
    // Shaders — no tree-sitter grammar yet, so these fall through
    // to `build_fallback_tree` and get indexed at text level. That's
    // a quality-of-life win on engines like Unreal where ~3K shader
    // files would otherwise be entirely invisible to the indexer.
    // Rich symbol extraction (cbuffer, Texture2D, [numthreads(...)],
    // etc.) is a follow-up via a Logos lexer.
    // HLSL — Microsoft / Direct3D / Unreal Engine `*.usf`+`*.ush`
    "hlsl",
    "usf",
    "ush",
    "fx",
    "fxh",
    "shader",
    // GLSL — OpenGL / Vulkan
    "glsl",
    "vert",
    "frag",
    "geom",
    "comp",
    "tesc",
    "tese",
    "mesh",
    "task",
    "rgen",
    "rmiss",
    "rchit",
    "rahit",
    "rint",
    "rcall",
    // Metal Shading Language — Apple
    "metal",
    // WebGPU Shading Language
    "wgsl",
    // --- Maximal coverage expansion (registered grammars) ---
    // CSS / SCSS
    "css",
    "scss",
    // GraphQL
    "graphql",
    "gql",
    // Groovy / Gradle
    "groovy",
    "gradle",
    // Nix
    "nix",
    // Erlang
    "erl",
    "hrl",
    // PowerShell
    "ps1",
    "psm1",
    "psd1",
    // Solidity
    "sol",
    // Objective-C / Objective-C++
    "m",
    "mm",
    // Julia
    "jl",
    // CMake (also `CMakeLists.txt` via filename detection)
    "cmake",
    // Make (also `Makefile` via filename detection)
    "mk",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_registry_has_rust() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("rs"));
        assert!(reg.language_for_extension("rs").is_some());
        assert!(reg.language_by_name("rust").is_some());
    }

    #[test]
    fn language_name_for_extension() {
        let reg = GrammarRegistry::global();
        assert_eq!(reg.language_name_for_extension("rs"), Some("rust"));
    }

    #[test]
    fn unsupported_extension_returns_none() {
        let reg = GrammarRegistry::global();
        assert!(reg.language_for_extension("xyz").is_none());
        assert!(reg.language_name_for_extension("xyz").is_none());
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("py"));
        assert!(reg.supports_extension("pyi"));
        assert!(reg.language_by_name("python").is_some());
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn typescript_and_tsx_registered() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("ts"));
        assert!(reg.supports_extension("tsx"));
        assert!(reg.language_by_name("typescript").is_some());
        assert!(reg.language_by_name("tsx").is_some());
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn go_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("go"));
        assert!(reg.language_by_name("go").is_some());
    }

    #[cfg(feature = "lang-java")]
    #[test]
    fn java_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("java"));
        assert!(reg.language_by_name("java").is_some());
    }

    #[test]
    fn all_code_extensions_is_nonempty() {
        assert!(ALL_CODE_EXTENSIONS.len() > 30);
    }

    #[test]
    fn coverage_expansion_extensions_recognized() {
        for ext in &[
            "css", "scss", "graphql", "gql", "groovy", "gradle", "nix", "erl", "hrl", "ps1",
            "psm1", "psd1", "sol", "m", "mm", "jl", "cmake", "mk",
        ] {
            assert!(
                ALL_CODE_EXTENSIONS.contains(ext),
                "coverage extension '{ext}' missing from ALL_CODE_EXTENSIONS"
            );
        }
    }

    #[cfg(feature = "lang-css")]
    #[test]
    fn css_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("css"));
        assert!(reg.supports_extension("scss"));
        assert!(reg.language_by_name("css").is_some());
    }

    #[cfg(feature = "lang-graphql")]
    #[test]
    fn graphql_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("graphql"));
        assert!(reg.supports_extension("gql"));
        assert!(reg.language_by_name("graphql").is_some());
    }

    #[cfg(feature = "lang-groovy")]
    #[test]
    fn groovy_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("groovy"));
        assert!(reg.supports_extension("gradle"));
        assert!(reg.language_by_name("groovy").is_some());
    }

    #[cfg(feature = "lang-nix")]
    #[test]
    fn nix_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("nix"));
        assert!(reg.language_by_name("nix").is_some());
    }

    #[cfg(feature = "lang-erlang")]
    #[test]
    fn erlang_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("erl"));
        assert!(reg.language_by_name("erlang").is_some());
    }

    #[cfg(feature = "lang-powershell")]
    #[test]
    fn powershell_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("ps1"));
        assert!(reg.language_by_name("powershell").is_some());
    }

    #[cfg(feature = "lang-solidity")]
    #[test]
    fn solidity_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("sol"));
        assert!(reg.language_by_name("solidity").is_some());
    }

    #[cfg(feature = "lang-objc")]
    #[test]
    fn objc_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("m"));
        assert!(reg.supports_extension("mm"));
        assert!(reg.language_by_name("objc").is_some());
    }

    #[cfg(feature = "lang-julia")]
    #[test]
    fn julia_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("jl"));
        assert!(reg.language_by_name("julia").is_some());
    }

    #[cfg(feature = "lang-cmake")]
    #[test]
    fn cmake_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("cmake"));
        assert!(reg.language_by_name("cmake").is_some());
    }

    #[cfg(feature = "lang-make")]
    #[test]
    fn make_registered_when_feature_enabled() {
        let reg = GrammarRegistry::global();
        assert!(reg.supports_extension("mk"));
        assert!(reg.language_by_name("make").is_some());
    }

    #[test]
    fn assembly_extensions_recognized() {
        for ext in &["asm", "s", "S", "inc"] {
            assert!(
                ALL_CODE_EXTENSIONS.contains(ext),
                "assembly extension '{ext}' missing from ALL_CODE_EXTENSIONS"
            );
        }
    }

    #[test]
    fn grammar_metadata_accessible() {
        let reg = GrammarRegistry::global();
        let g = reg.grammar("rust").expect("rust grammar");
        assert_eq!(g.name, "rust");
        assert!(g.extensions.contains(&"rs"));
    }

    #[test]
    fn language_count_at_least_one() {
        let reg = GrammarRegistry::global();
        assert!(reg.language_count() >= 1);
    }
}
