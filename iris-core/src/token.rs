//! Token counting utility for accurate budget tracking.
//!
//! Wraps the `tiktoken-rs` crate to provide cl100k_base-compatible token
//! counting for all content units (sections, claims, summaries). This
//! encoding is used by GPT-4, GPT-3.5-turbo, and text-embedding-ada-002,
//! making it a reasonable default for budget estimation across models.

use std::sync::OnceLock;

use tiktoken_rs::CoreBPE;

/// Global cached encoder instance. Initialized once on first use.
fn encoder() -> &'static CoreBPE {
    static ENCODER: OnceLock<CoreBPE> = OnceLock::new();
    ENCODER.get_or_init(|| {
        tiktoken_rs::cl100k_base().expect("failed to initialize cl100k_base encoder")
    })
}

/// Count the number of tokens in a text string using `cl100k_base` encoding.
///
/// # Examples
///
/// ```
/// use iris_core::token::count_tokens;
///
/// let n = count_tokens("hello world");
/// assert!(n > 0);
/// ```
#[must_use]
pub fn count_tokens(text: &str) -> usize {
    encoder().encode_with_special_tokens(text).len()
}

/// Count tokens and check whether the count exceeds a budget limit.
///
/// Returns `(token_count, within_budget)` where `within_budget` is `true`
/// if `token_count <= limit`.
///
/// # Examples
///
/// ```
/// use iris_core::token::count_tokens_with_limit;
///
/// let (count, within) = count_tokens_with_limit("hello world", 100);
/// assert!(within);
/// assert!(count <= 100);
/// ```
#[must_use]
pub fn count_tokens_with_limit(text: &str, limit: usize) -> (usize, bool) {
    let count = count_tokens(text);
    (count, count <= limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_is_zero_tokens() {
        assert_eq!(count_tokens(""), 0);
    }

    #[test]
    fn simple_text_has_expected_count() {
        // "hello world" is 2 tokens in cl100k_base
        let count = count_tokens("hello world");
        assert_eq!(count, 2);
    }

    #[test]
    fn longer_text_has_more_tokens() {
        let short = count_tokens("hello");
        let long = count_tokens("hello world, this is a longer sentence with many words");
        assert!(long > short);
    }

    #[test]
    fn within_budget_returns_true() {
        let (count, within) = count_tokens_with_limit("hello world", 100);
        assert_eq!(count, 2);
        assert!(within);
    }

    #[test]
    fn exceeds_budget_returns_false() {
        let (count, within) = count_tokens_with_limit("hello world", 1);
        assert_eq!(count, 2);
        assert!(!within);
    }

    #[test]
    fn exact_budget_returns_true() {
        let (count, within) = count_tokens_with_limit("hello world", 2);
        assert_eq!(count, 2);
        assert!(within);
    }

    #[test]
    fn unicode_text_counted() {
        let count = count_tokens("日本語テスト");
        assert!(count > 0);
    }

    #[test]
    fn code_snippet_counted() {
        let code = "fn main() {\n    println!(\"hello\");\n}";
        let count = count_tokens(code);
        assert!(count > 0);
    }

    #[test]
    fn encoder_is_reused_across_calls() {
        // Calling multiple times should not panic (tests OnceLock reuse)
        let a = count_tokens("first call");
        let b = count_tokens("second call");
        assert!(a > 0);
        assert!(b > 0);
    }
}
