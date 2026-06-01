//! FE — TypeScript comprehensive extraction edge cases.
//!
//! Firm positive assertions cover what the extractor gets right across the
//! constructs real TS uses. Two real gaps are pinned as flip-when-fixed
//! baselines: interface **declaration merging** (a second `interface X` is
//! dropped) and **ambient `declare`** declarations (not extracted). Function
//! overload-signature collapse is the same root as the C++ overload gap
//! (tracked: f-overload-symbols-collapse).

mod langtest;

use langtest::{IngestedProject, assert_range_invariant, assert_symbol};

fn ts_kitchen_sink() -> &'static str {
    r#"// Type alias + generic.
export type Id = string;
export type Pair<T> = { first: T; second: T };

// Interface with a method + an interface that merges with itself.
export interface Shape {
    area(): number;
}
export interface Shape {
    name: string;
}

// Const enum + regular enum.
export enum Color { Red, Green, Blue }
export const enum Direction { North, South }

// Abstract class + generic class implementing an interface.
export abstract class Base {
    abstract describe(): string;
}

export class Box<T> extends Base implements Shape {
    constructor(public value: T) { super(); }
    area(): number { return 0; }
    describe(): string { return "box"; }
}

// Arrow-const function.
export const double = (n: number): number => n * 2;

// Function with overload signatures.
export function parse(x: string): number;
export function parse(x: number): string;
export function parse(x: string | number): string | number {
    return typeof x === "string" ? Number(x) : String(x);
}

// Decorated class (experimental decorators).
function sealed(target: Function) {}

@sealed
export class Service {
    run(): void {}
}

// Namespace with a nested function (declaration-merging surface).
export namespace util {
    export function helper(): number { return 42; }
}

// Ambient declaration.
declare const VERSION: string;
"#
}

async fn sink() -> IngestedProject {
    IngestedProject::from_files(&[("kitchen.ts", ts_kitchen_sink())]).await
}

#[tokio::test]
async fn ts_type_aliases_and_generics() {
    let proj = sink().await;
    let _id = assert_symbol(&proj, "Id", "type", "kitchen.ts").await;
    // Generic type alias — the name is the bare identifier, no `<T>` suffix.
    let _pair = assert_symbol(&proj, "Pair", "type", "kitchen.ts").await;
}

#[tokio::test]
async fn ts_interface_maps_to_trait() {
    let proj = sink().await;
    let shape = assert_symbol(&proj, "Shape", "trait", "kitchen.ts").await;
    assert_range_invariant(&shape);
}

#[tokio::test]
async fn ts_enum_and_const_enum() {
    let proj = sink().await;
    let _color = assert_symbol(&proj, "Color", "enum", "kitchen.ts").await;
    let _dir = assert_symbol(&proj, "Direction", "enum", "kitchen.ts").await; // const enum
}

#[tokio::test]
async fn ts_abstract_and_generic_class() {
    let proj = sink().await;
    let base = assert_symbol(&proj, "Base", "struct", "kitchen.ts").await; // abstract class
    assert_range_invariant(&base);
    // Generic class — name is `Box`, not `Box<T>`.
    let boxc = assert_symbol(&proj, "Box", "struct", "kitchen.ts").await;
    assert_range_invariant(&boxc);
    assert!(boxc.line_end > boxc.line_start, "Box is multi-line");
}

#[tokio::test]
async fn ts_arrow_const_and_overload_impl() {
    let proj = sink().await;
    // Arrow function assigned to a const surfaces as a `const`.
    let _double = assert_symbol(&proj, "double", "const", "kitchen.ts").await;
    // Overloaded function: the implementation signature is the surviving fn.
    let _parse = assert_symbol(&proj, "parse", "function", "kitchen.ts").await;
}

#[tokio::test]
async fn ts_decorated_class_and_namespace() {
    let proj = sink().await;
    // A decorator does not break class extraction.
    let _service = assert_symbol(&proj, "Service", "struct", "kitchen.ts").await;
    // Namespace → module; its exported member is extracted too.
    let _util = assert_symbol(&proj, "util", "module", "kitchen.ts").await;
    let _helper = assert_symbol(&proj, "helper", "function", "kitchen.ts").await;
}

// ── Pinned gap baselines (flip when fixed) ────────────────────────────────

/// BASELINE: interface declaration merging drops the second declaration.
///
/// `interface Shape {…}` declared twice should merge into one logical type,
/// but the extractor stores a single `Shape` symbol (the first), losing the
/// second declaration's members. Same id-collision root as overload collapse.
#[tokio::test]
async fn ts_interface_declaration_merging_collapses_baseline() {
    let proj = sink().await;
    let shapes = proj.symbols_named("Shape").await;
    assert_eq!(
        shapes.len(),
        1,
        "interface declaration merging now yields {} `Shape` symbols \
         (expected the 1-symbol baseline). If merging is handled, assert the \
         merged members instead.",
        shapes.len(),
    );
}

/// BASELINE: ambient `declare const VERSION: string;` is not extracted.
///
/// Ambient declarations (`declare const/let/function/class`, `.d.ts` bodies)
/// are skipped, so `VERSION` produces no symbol. Tracked for a future fix.
#[tokio::test]
async fn ts_ambient_declare_not_extracted_baseline() {
    let proj = sink().await;
    let version = proj.symbols_named("VERSION").await;
    assert!(
        version.is_empty(),
        "ambient `declare const VERSION` is now extracted ({} symbol(s)) — \
         promote this baseline to a positive ambient-declaration assertion.",
        version.len(),
    );
}
