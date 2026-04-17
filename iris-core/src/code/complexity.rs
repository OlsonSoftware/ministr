//! Cyclomatic complexity calculation from tree-sitter AST nodes.
//!
//! Computes the cyclomatic complexity of a function by counting decision
//! points in its body: branches (`if`, `match` arms), loops (`while`, `for`,
//! `loop`), short-circuit operators (`&&`, `||`), and early-return error
//! handling (`?`).
//!
//! The formula is: **CC = 1 + `decision_points`**.

/// Compute the cyclomatic complexity of a function node.
///
/// Walks the function body subtree and counts decision points. Returns 1
/// (base complexity) if the node has no body or is not a function.
///
/// # Decision points counted
///
/// - `if_expression` (each `if` / `else if`)
/// - `match_expression` arms (N arms → N−1 decisions)
/// - `while_expression`, `for_expression`, `loop_expression`
/// - `&&` and `||` binary operators
/// - `?` operator (`try_expression`)
///
/// # Examples
///
/// ```
/// use iris_core::code::{AstParser, cyclomatic_complexity};
///
/// let mut parser = AstParser::new();
/// let source = b"fn simple() { let x = 1; }";
/// let tree = parser.parse(source).unwrap();
/// let root = tree.root_node();
/// let func = root.child(0).unwrap();
/// assert_eq!(cyclomatic_complexity(&func, source), 1);
/// ```
#[must_use]
pub fn cyclomatic_complexity(node: &tree_sitter::Node, source: &[u8]) -> u32 {
    let Some(body) = node.child_by_field_name("body") else {
        return 1;
    };
    1 + count_decision_points(&body, source)
}

/// Recursively count decision points in a subtree.
fn count_decision_points(node: &tree_sitter::Node, source: &[u8]) -> u32 {
    let mut count = 0;

    match node.kind() {
        // Each `if` is a decision point (covers both `if` and `else if`).
        // Loops are decision points (enter vs skip/exit).
        // `?` operator — early return on error.
        "if_expression" | "while_expression" | "for_expression" | "loop_expression"
        | "try_expression" => count += 1,

        // Each match arm beyond the first is a decision point.
        // Arms live inside a `match_block` child, so we count them there.
        "match_block" => {
            let arm_count = count_children_of_kind(node, "match_arm");
            count += arm_count.saturating_sub(1);
        }

        // Short-circuit boolean operators
        "binary_expression" => {
            if let Some(op) = node.child_by_field_name("operator") {
                let op_text = op.utf8_text(source).unwrap_or("");
                if op_text == "&&" || op_text == "||" {
                    count += 1;
                }
            }
        }

        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decision_points(&child, source);
    }

    count
}

/// Count direct children of a specific node kind.
fn count_children_of_kind(node: &tree_sitter::Node, kind: &str) -> u32 {
    let mut count = 0u32;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::AstParser;

    fn complexity_of(source: &str) -> u32 {
        let mut parser = AstParser::new();
        let tree = parser.parse(source.as_bytes()).unwrap();
        let root = tree.root_node();
        // Find the first function_item child
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "function_item" {
                return cyclomatic_complexity(&child, source.as_bytes());
            }
        }
        panic!("no function_item found in source");
    }

    #[test]
    fn straight_line_code() {
        assert_eq!(complexity_of("fn f() { let x = 1; let y = 2; }"), 1);
    }

    #[test]
    fn single_if() {
        assert_eq!(complexity_of("fn f(x: bool) { if x { } }"), 2);
    }

    #[test]
    fn if_else() {
        // if/else is still one decision point (one branch)
        assert_eq!(complexity_of("fn f(x: bool) { if x { } else { } }"), 2);
    }

    #[test]
    fn if_else_if_else() {
        // Two decision points: `if` and `else if`
        let src = "fn f(x: i32) { if x > 0 { } else if x < 0 { } else { } }";
        assert_eq!(complexity_of(src), 3);
    }

    #[test]
    fn while_loop() {
        assert_eq!(complexity_of("fn f() { while true { break; } }"), 2);
    }

    #[test]
    fn for_loop() {
        assert_eq!(complexity_of("fn f() { for i in 0..10 { let _ = i; } }"), 2);
    }

    #[test]
    fn loop_expression() {
        assert_eq!(complexity_of("fn f() { loop { break; } }"), 2);
    }

    #[test]
    fn match_two_arms() {
        let src = r"fn f(x: i32) { match x { 0 => {}, _ => {} } }";
        // 2 arms → 1 decision point → CC = 2
        assert_eq!(complexity_of(src), 2);
    }

    #[test]
    fn match_four_arms() {
        let src = r"fn f(x: i32) { match x { 0 => {}, 1 => {}, 2 => {}, _ => {} } }";
        // 4 arms → 3 decision points → CC = 4
        assert_eq!(complexity_of(src), 4);
    }

    #[test]
    fn logical_and_or() {
        let src = "fn f(a: bool, b: bool, c: bool) { if a && b || c { } }";
        // 1 (if) + 1 (&&) + 1 (||) = 3 decision points → CC = 4
        assert_eq!(complexity_of(src), 4);
    }

    #[test]
    fn try_operator() {
        let src = r"fn f() -> Result<(), ()> { let x = something()?; Ok(()) }";
        // 1 `?` → CC = 2
        assert_eq!(complexity_of(src), 2);
    }

    #[test]
    fn nested_control_flow() {
        let src = r"
fn f(items: Vec<i32>) {
    for item in items {
        if item > 0 {
            match item {
                1 => {},
                2 => {},
                _ => {},
            }
        }
    }
}
";
        // for (1) + if (1) + match 3 arms (2) = 4 → CC = 5
        assert_eq!(complexity_of(src), 5);
    }

    #[test]
    fn no_body_returns_one() {
        // A function declaration without body (e.g. in a trait)
        let mut parser = AstParser::new();
        let source = b"trait T { fn method(&self); }";
        let tree = parser.parse(source).unwrap();
        let root = tree.root_node();
        // Find the trait, then the function inside
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "trait_item"
                && let Some(body) = child.child_by_field_name("body")
            {
                let mut body_cursor = body.walk();
                for body_child in body.children(&mut body_cursor) {
                    if body_child.kind() == "function_signature_item" {
                        assert_eq!(cyclomatic_complexity(&body_child, source), 1);
                        return;
                    }
                }
            }
        }
        panic!("did not find function_signature_item");
    }

    #[test]
    fn complex_real_world_function() {
        let src = r"
fn process(data: &[u8]) -> Result<String, Error> {
    if data.is_empty() {
        return Err(Error::Empty);
    }

    let parsed = parse(data)?;

    let mut result = String::new();
    for item in &parsed {
        if item.is_valid() && item.is_active() {
            match item.kind() {
                Kind::A => result.push('a'),
                Kind::B => result.push('b'),
                _ => {}
            }
        } else if item.is_fallback() || item.is_legacy() {
            result.push('?');
        }
    }
    Ok(result)
}
";
        // if (1) + ? (1) + for (1) + if (1) + && (1) + match 3 arms (2)
        // + else if (1) + || (1) = 9 → CC = 10
        assert_eq!(complexity_of(src), 10);
    }
}
