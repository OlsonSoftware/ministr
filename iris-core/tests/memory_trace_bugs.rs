//! Regression guards for bugs found in the FSRS memory-tracker trace.

use iris_core::session::memory::{AccessRating, MemoryState, MemoryTracker};

/// MH1 regression — `record_access` must seed new entries according to
/// the supplied rating. Before the fix it silently dropped `rating` on
/// the first access, so `Easy` produced identical state to `Good`.
#[test]
fn mh1_easy_rating_seeds_lower_difficulty_on_new_entry() {
    let mut tracker = MemoryTracker::new();

    tracker.record_access("easy", 0, AccessRating::Easy);
    tracker.record_access("good", 0, AccessRating::Good);

    let easy = tracker.get_state("easy").unwrap();
    let good = tracker.get_state("good").unwrap();

    assert!(
        easy.difficulty < good.difficulty,
        "Easy rating must reduce initial difficulty below Good; easy={}, good={}",
        easy.difficulty,
        good.difficulty
    );
    assert!((good.difficulty - 0.3).abs() < f64::EPSILON);

    // A first access is still one access regardless of rating.
    assert_eq!(easy.access_count, 1);
    assert_eq!(good.access_count, 1);
}

/// MH1 regression — `Again` on a first access is semantically
/// incoherent (there's nothing to forget). The fix falls back to `Good`
/// defaults rather than silently accepting or corrupting state.
#[test]
fn mh1_again_rating_on_new_entry_falls_back_to_good_defaults() {
    let mut tracker = MemoryTracker::new();
    tracker.record_access("again", 0, AccessRating::Again);

    let state = tracker.get_state("again").unwrap();
    // Same as a Good new entry.
    let expected = MemoryState::new(0);
    assert!((state.stability - expected.stability).abs() < f64::EPSILON);
    assert!((state.difficulty - expected.difficulty).abs() < f64::EPSILON);
    assert_eq!(state.access_count, expected.access_count);
}

/// MH2 regression — negative `hours_since_last_session` must NOT blow
/// stability up to `+inf` (divide-by-zero) or flip its sign. Clock skew
/// is treated as "no time elapsed".
#[test]
fn mh2_negative_hours_are_clamped_to_zero() {
    let original_stability = 4.0;
    let states = vec![(
        "s1".to_string(),
        MemoryState {
            stability: original_stability,
            difficulty: 0.3,
            last_access_turn: 10,
            access_count: 5,
        },
    )];

    let tracker = MemoryTracker::load_from_persisted(states, -168.0);
    let state = tracker.get_state("s1").unwrap();

    assert!(
        state.stability.is_finite(),
        "stability must stay finite under negative hours; got {}",
        state.stability
    );
    assert!(
        (state.stability - original_stability).abs() < f64::EPSILON,
        "negative hours → treated as zero → stability unchanged; got {}",
        state.stability
    );
}

/// MH2 regression — a very large negative hours value (far below -168h,
/// where the naive formula would flip the sign) also stays well-behaved.
#[test]
fn mh2_large_negative_hours_stay_bounded() {
    let states = vec![(
        "s1".to_string(),
        MemoryState {
            stability: 2.5,
            difficulty: 0.4,
            last_access_turn: 0,
            access_count: 3,
        },
    )];

    let tracker = MemoryTracker::load_from_persisted(states, -10_000.0);
    let state = tracker.get_state("s1").unwrap();

    assert!(state.stability.is_finite());
    assert!(state.stability > 0.0);
    assert!(state.stability <= 2.5);
}
