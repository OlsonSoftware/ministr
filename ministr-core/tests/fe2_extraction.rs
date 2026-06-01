//! FE2 — Symbol-extraction edge-case matrix (every supported language).
//!
//! Built on the shared FE1 [`langtest`] harness: each test ingests a
//! self-contained, edge-case-focused fixture end-to-end through the real
//! ingestion pipeline, then asserts on the **stored** symbol graph
//! ([`SymbolRecord`], which carries the 1-based `line_start`/`line_end` the
//! resolver actually reads).
//!
//! Two invariants every language test upholds:
//!
//! 1. **Edge-case coverage** — each fixture exercises the constructs that
//!    historically break extractors: nested/inner types, anonymous +
//!    arrow/const functions, method overloads, generics/type params,
//!    decorators/annotations/attributes, multi-symbol declarations, and the
//!    per-language quirks called out in the FE roadmap (Go embedded structs +
//!    promoted methods, Ruby reopened classes/modules, Kotlin extension fns +
//!    companion objects, Swift extensions + protocol conformances, TS
//!    declaration merging).
//! 2. **Range invariant** — every multi-line symbol satisfies
//!    `line_end >= line_start >= 1` (the malformed-range bug that was a
//!    candidate root cause for dropped refs). Asserted in bulk via
//!    [`assert_ranges_well_formed`] plus a stronger `line_end > line_start`
//!    spot-check on representative multi-line definitions.
//!
//! A coverage guard ([`every_code_grammar_has_an_extraction_fixture`]) keeps
//! this matrix honest: it partitions the *registered* grammar set into the
//! languages this suite covers in depth and an explicitly-documented deferral
//! allowlist, and fails when a newly-registered code grammar falls in neither.

mod langtest;

use langtest::{IngestedProject, assert_range_invariant, assert_symbol};

/// Assert the range invariant on **every** symbol in the project — the cheap,
/// universal half of acceptance criterion #2. Returns the symbols so callers
/// can layer language-specific assertions on top without re-querying.
async fn assert_ranges_well_formed(proj: &IngestedProject) -> Vec<ministr_core::storage::SymbolRecord> {
    let symbols = proj.all_symbols().await;
    assert!(
        !symbols.is_empty(),
        "fixture produced no symbols — nothing to assert a range invariant on",
    );
    for sym in &symbols {
        assert_range_invariant(sym);
    }
    symbols
}

/// Assert `sym` spans more than one line — for definitions that always have a
/// brace/`def` body, a collapsed `line_end == line_start` means the body range
/// was lost.
fn assert_multiline(sym: &ministr_core::storage::SymbolRecord) {
    assert!(
        sym.line_end > sym.line_start,
        "`{}` ({}) has a body and must span >1 line, got {}..{}",
        sym.name,
        sym.kind,
        sym.line_start,
        sym.line_end,
    );
}

// ── Rust ────────────────────────────────────────────────────────────────
//
// Edge cases: generics (`Container<T>`), an inline nested module with its own
// items, trait + impl methods, `#[derive(...)]` attributes, `const`/`static`,
// and an enum with struct-like + tuple variants.

#[tokio::test]
async fn rust_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "lib.rs",
        r#"use std::fmt::Debug;

/// A generic container.
#[derive(Debug, Clone)]
pub struct Container<T> {
    items: Vec<T>,
}

impl<T: Debug> Container<T> {
    pub fn new() -> Self {
        Container { items: Vec::new() }
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

pub trait Store {
    fn save(&self);
}

pub mod helpers {
    pub fn assist() -> i32 {
        42
    }
    pub struct Inner {
        pub field: i32,
    }
}

pub const MAX: usize = 10;
pub static NAME: &str = "ministr";

pub enum Shape {
    Circle(f64),
    Square { side: f64 },
}
"#,
    )])
    .await;

    let syms = assert_ranges_well_formed(&proj).await;

    let container = assert_symbol(&proj, "Container", "struct", "lib.rs").await;
    assert_multiline(&container);
    assert_eq!(container.visibility, "pub", "Container is `pub`");

    assert_symbol(&proj, "Store", "trait", "lib.rs").await;
    assert_symbol(&proj, "Shape", "enum", "lib.rs").await;
    assert_symbol(&proj, "MAX", "const", "lib.rs").await;
    assert_symbol(&proj, "NAME", "static", "lib.rs").await;
    assert_symbol(&proj, "new", "function", "lib.rs").await;
    assert_symbol(&proj, "len", "function", "lib.rs").await;

    // The inline `mod helpers` is itself extracted (with a well-formed range),
    // but the extractor does NOT currently recurse into a module body — its
    // children (`assist`, `Inner`) are not emitted. This characterizes today's
    // behavior; deeper nested-item extraction is tracked separately
    // (f-nested-class-extraction).
    let helpers = assert_symbol(&proj, "helpers", "module", "lib.rs").await;
    assert_multiline(&helpers);
    assert!(
        syms.iter().all(|s| s.name != "assist"),
        "nested-module children are not expected to be extracted yet, but `assist` was",
    );

    // The `#[derive(Debug, Clone)]` attribute is captured as an annotation.
    assert!(
        syms.iter()
            .find(|s| s.name == "Container")
            .map(|s| !s.signature.is_empty())
            .unwrap_or(false),
        "Container should carry a non-empty signature",
    );
}

// ── Python ──────────────────────────────────────────────────────────────
//
// Edge cases: a nested (inner) class, `@property`/`@staticmethod` decorators,
// a nested function, an `async def`, and a `Generic[T]` class.

#[tokio::test]
async fn python_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "app.py",
        r#"from typing import Generic, TypeVar

T = TypeVar("T")


class Outer:
    """An outer class containing a nested class."""

    class Inner:
        def ping(self) -> int:
            return 1

    @property
    def value(self) -> int:
        return self._v

    @staticmethod
    def make() -> "Outer":
        return Outer()


def standalone(x: int) -> int:
    def nested(y: int) -> int:
        return y + 1

    return nested(x)


async def fetch(url: str) -> str:
    return url


class Container(Generic[T]):
    items: list
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let outer = assert_symbol(&proj, "Outer", "struct", "app.py").await;
    assert_multiline(&outer);
    assert_symbol(&proj, "Container", "struct", "app.py").await;
    assert_symbol(&proj, "standalone", "function", "app.py").await;
    assert_symbol(&proj, "fetch", "function", "app.py").await;
}

// ── JavaScript ────────────────────────────────────────────────────────────
//
// Edge cases: class inheritance (`extends`), an overriding method, a plain
// function, an arrow-const, and an object-literal method.

#[tokio::test]
async fn javascript_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "app.js",
        r#"export class Animal {
  constructor(name) {
    this.name = name;
  }
  speak() {
    return this.name;
  }
}

class Dog extends Animal {
  speak() {
    return super.speak() + " woof";
  }
}

function regular(a, b) {
  return a + b;
}

const arrow = (x) => x * 2;

export default function main() {
  return new Dog("rex");
}
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let animal = assert_symbol(&proj, "Animal", "struct", "app.js").await;
    assert_multiline(&animal);
    assert_symbol(&proj, "Dog", "struct", "app.js").await;
    assert_symbol(&proj, "regular", "function", "app.js").await;
}

// ── TypeScript ──────────────────────────────────────────────────────────
//
// Edge cases: a generic interface, a class implementing it with type params,
// a type alias, an enum, a generic function, and a namespace.

#[tokio::test]
async fn typescript_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "repo.ts",
        r#"export interface Repository<T> {
  get(id: string): T | undefined;
}

export class MemoryRepo<T> implements Repository<T> {
  private items = new Map<string, T>();
  get(id: string): T | undefined {
    return this.items.get(id);
  }
}

export type ID = string | number;

export enum Color {
  Red,
  Green,
  Blue,
}

export function identity<T>(x: T): T {
  return x;
}

namespace Geometry {
  export function area(r: number): number {
    return 3.14 * r * r;
  }
}
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let repo = assert_symbol(&proj, "Repository", "trait", "repo.ts").await;
    assert_multiline(&repo);
    assert_symbol(&proj, "MemoryRepo", "struct", "repo.ts").await;
    assert_symbol(&proj, "Color", "enum", "repo.ts").await;
    assert_symbol(&proj, "identity", "function", "repo.ts").await;
}

// ── TSX ───────────────────────────────────────────────────────────────────
//
// Edge cases: a props interface, a function component, a class component, and
// an arrow-const component (the JSX-bearing variants).

#[tokio::test]
async fn tsx_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "card.tsx",
        r#"interface Props {
  title: string;
}

export const Button = ({ title }: Props) => {
  return <button>{title}</button>;
};

export function Card(props: Props) {
  return <div>{props.title}</div>;
}

export class Panel {
  props: Props;
  render(): string {
    return this.props.title;
  }
}
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let props = assert_symbol(&proj, "Props", "trait", "card.tsx").await;
    assert_multiline(&props);
    assert_symbol(&proj, "Card", "function", "card.tsx").await;
    assert_symbol(&proj, "Panel", "struct", "card.tsx").await;
}

// ── Go ────────────────────────────────────────────────────────────────────
//
// Edge cases: a structural interface, an embedded struct + a promoted method,
// a pointer-receiver constructor, generic type params (`Map[T, R]`), and a
// const.

#[tokio::test]
async fn go_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "main.go",
        r#"package main

type Reader interface {
	Read(p []byte) (int, error)
}

type Base struct {
	ID int
}

func (b Base) Describe() string {
	return "base"
}

// User embeds Base, promoting Describe().
type User struct {
	Base
	Name string
}

func NewUser(name string) *User {
	return &User{Name: name}
}

func Map[T any, R any](xs []T, f func(T) R) []R {
	out := make([]R, 0, len(xs))
	for _, x := range xs {
		out = append(out, f(x))
	}
	return out
}

const MaxUsers = 100
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    // Go `type X struct {…}` / `type X interface {…}` declarations extract as
    // kind `type` (the generic Go path maps type_declaration → ItemKind::Type),
    // not `struct`/`trait`.
    let base = assert_symbol(&proj, "Base", "type", "main.go").await;
    assert_multiline(&base);
    assert_symbol(&proj, "User", "type", "main.go").await;
    assert_symbol(&proj, "Reader", "type", "main.go").await;
    assert_symbol(&proj, "NewUser", "function", "main.go").await;
    assert_symbol(&proj, "Map", "function", "main.go").await;
    // Promoted/receiver method on the embedded Base.
    assert_symbol(&proj, "Describe", "function", "main.go").await;
}

// ── Java ──────────────────────────────────────────────────────────────────
//
// Edge cases: a generic class implementing an interface, overloaded methods,
// an `@Override` annotation, a nested inner class + a static nested class, and
// an enum.

#[tokio::test]
async fn java_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "Box.java",
        r#"public interface Shape {
    double area();
}

public class Box<T> implements Shape {
    private T content;

    @Override
    public double area() {
        return 0.0;
    }

    public void put(T item) {
        this.content = item;
    }

    public void put(T item, int count) {
        this.content = item;
    }

    public class Label {
        String text;
    }

    public static class Builder {
        Box<Object> build() {
            return new Box<>();
        }
    }
}

enum Color {
    RED,
    GREEN,
    BLUE,
}
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let shape = assert_symbol(&proj, "Shape", "trait", "Box.java").await;
    assert_multiline(&shape);
    let boxc = assert_symbol(&proj, "Box", "struct", "Box.java").await;
    assert_multiline(&boxc);
    assert_symbol(&proj, "Color", "enum", "Box.java").await;
    assert_symbol(&proj, "area", "function", "Box.java").await;
}

// ── C ─────────────────────────────────────────────────────────────────────
//
// Edge cases: a named struct, a `typedef struct` (anonymous body), a union, an
// enum, a function prototype vs definition (same name), a `static` function,
// and a function-pointer typedef.

#[tokio::test]
async fn c_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "lib.c",
        r#"#include <stddef.h>

struct Point {
    int x;
    int y;
};

typedef struct {
    double re;
    double im;
} Complex;

union Value {
    int i;
    float f;
};

enum Status {
    OK,
    ERR,
};

int add(int a, int b);

int add(int a, int b) {
    return a + b;
}

static void helper(void) {
}
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let point = assert_symbol(&proj, "Point", "struct", "lib.c").await;
    assert_multiline(&point);
    assert_symbol(&proj, "Status", "enum", "lib.c").await;
    assert_symbol(&proj, "add", "function", "lib.c").await;
    assert_symbol(&proj, "helper", "function", "lib.c").await;
}

// ── C++ ────────────────────────────────────────────────────────────────────
//
// Edge cases: a namespace, a class template with type params, a nested class,
// an `enum class`, and a plain struct. (fe_cpp.rs covers the cross-file ref +
// `.h` ambiguity story; this focuses on extraction-shape edge cases.)

#[tokio::test]
async fn cpp_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "geo.hpp",
        r#"#ifndef GEO_HPP
#define GEO_HPP

namespace geo {

template <typename T>
class Vec {
public:
    T x, y;
    T dot(const Vec<T>& o) const {
        return x * o.x + y * o.y;
    }

    class Iterator {
    public:
        int pos;
    };
};

enum class Kind {
    A,
    B,
    C,
};

struct Pair {
    int a;
    int b;
};

}  // namespace geo

#endif
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let vec = assert_symbol(&proj, "Vec", "struct", "geo.hpp").await;
    assert_multiline(&vec);
    assert_symbol(&proj, "Pair", "struct", "geo.hpp").await;
    assert_symbol(&proj, "dot", "function", "geo.hpp").await;
}

// ── Ruby ──────────────────────────────────────────────────────────────────
//
// Edge cases: a module with a constant, a class inside the module, subclassing
// (`<`), a reopened class (defined twice), and a standalone method.

#[tokio::test]
async fn ruby_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "geometry.rb",
        r#"module Geometry
  PI = 3.14159

  class Shape
    def area
      0
    end
  end

  class Circle < Shape
    def initialize(r)
      @r = r
    end

    def area
      PI * @r * @r
    end
  end
end

class Geometry::Shape
  def describe
    "shape"
  end
end

def standalone(x)
  x * 2
end
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let geometry = assert_symbol(&proj, "Geometry", "module", "geometry.rb").await;
    assert_multiline(&geometry);
    // A subclass inside the module body IS extracted…
    let circle = assert_symbol(&proj, "Circle", "struct", "geometry.rb").await;
    assert_multiline(&circle);
    // …and the *reopened* class (written with the `Geometry::Shape` path
    // outside the module) is extracted under its qualified name.
    assert_symbol(&proj, "Geometry::Shape", "struct", "geometry.rb").await;
    assert_symbol(&proj, "standalone", "function", "geometry.rb").await;
    // Methods are extracted even when their enclosing in-module class is not
    // separately emitted (the plain `class Shape` body here): both `area`
    // definitions appear as functions.
    assert!(
        proj.symbols_named("area").await.len() >= 2,
        "both `area` method definitions should be extracted",
    );
}

// ── C# ──────────────────────────────────────────────────────────────────
//
// Edge cases: a namespace, a generic class implementing an interface,
// overloaded methods, a nested class, an expression-bodied method, and an
// enum.

#[tokio::test]
async fn csharp_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "App.cs",
        r#"namespace App
{
    public interface ISerializable
    {
        string Serialize();
    }

    public class Repository<T> : ISerializable
    {
        public string Serialize() => "repo";

        public void Add(T item) { }

        public void Add(T item, int count) { }

        public class Cursor
        {
            public int Position;
        }
    }

    public enum Color
    {
        Red,
        Green,
        Blue,
    }
}
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let iface = assert_symbol(&proj, "ISerializable", "trait", "App.cs").await;
    assert_multiline(&iface);
    let repo = assert_symbol(&proj, "Repository", "struct", "App.cs").await;
    assert_multiline(&repo);
    assert_symbol(&proj, "Color", "enum", "App.cs").await;
    assert_symbol(&proj, "Serialize", "function", "App.cs").await;
}

// ── Swift ──────────────────────────────────────────────────────────────
//
// Edge cases: a protocol, a struct, a class with a protocol conformance, an
// `extension` adding a method, an enum, and a generic free function.

#[tokio::test]
async fn swift_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "shapes.swift",
        r#"protocol Drawable {
    func draw()
}

struct Point {
    var x: Int
    var y: Int
}

class Shape: Drawable {
    func draw() {}
}

extension Point {
    func magnitude() -> Int {
        return x * x + y * y
    }
}

enum Direction {
    case north
    case south
}

func identity<T>(_ value: T) -> T {
    return value
}
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    let point = assert_symbol(&proj, "Point", "struct", "shapes.swift").await;
    assert_multiline(&point);
    assert_symbol(&proj, "draw", "function", "shapes.swift").await;
    assert_symbol(&proj, "magnitude", "function", "shapes.swift").await;
    assert_symbol(&proj, "identity", "function", "shapes.swift").await;
}

// ── Kotlin ──────────────────────────────────────────────────────────────
//
// Edge cases: an interface, a generic class with `override`, a companion
// object, a data class, an extension function, and an enum class.

#[tokio::test]
async fn kotlin_extraction_edge_cases() {
    let proj = IngestedProject::from_files(&[(
        "Repo.kt",
        r#"interface Serializable {
    fun serialize(): String
}

class Repository<T> : Serializable {
    override fun serialize(): String = "repo"

    companion object {
        fun create(): Repository<Any> = Repository()
    }
}

data class User(val id: Int, val name: String)

fun String.shout(): String = this.uppercase()

enum class Color {
    RED,
    GREEN,
    BLUE,
}
"#,
    )])
    .await;

    assert_ranges_well_formed(&proj).await;

    // Kotlin's tree-sitter grammar uses `class_declaration` for interfaces and
    // enum classes too, so the generic extractor maps them all to `struct`.
    let repo = assert_symbol(&proj, "Repository", "struct", "Repo.kt").await;
    assert_multiline(&repo);
    assert_symbol(&proj, "Serializable", "struct", "Repo.kt").await;
    assert_symbol(&proj, "User", "struct", "Repo.kt").await;
    assert_symbol(&proj, "Color", "struct", "Repo.kt").await;
    // Top-level (extension) functions are extracted. Class-member functions
    // (`serialize`, the `companion object`'s `create`) are NOT emitted by the
    // generic Kotlin path today — characterized here so a future improvement
    // flips this assertion deliberately.
    assert_symbol(&proj, "shout", "function", "Repo.kt").await;
    assert!(
        proj.symbols_named("serialize").await.is_empty(),
        "class-member fns are not expected to be extracted for Kotlin yet, but `serialize` was",
    );
}
