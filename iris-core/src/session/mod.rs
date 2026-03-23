//! Session shadow and budget management for iris.
//!
//! This module implements the session intelligence layer — tracking what content
//! has been delivered to an agent, estimating context window usage, and managing
//! token budgets with pressure-based response compression.
//!
//! # Architecture
//!
//! - [`Session`] — tracks delivered content, access trajectory, and dedup state
//! - [`WindowEstimator`] — models context window fill with FIFO/LRU eviction
//! - [`BudgetTracker`] — threshold-based pressure levels driving response compression
//! - [`EvictionRanker`] — scores delivered items for eviction priority
//! - [`CompressionPipeline`] — multi-tier compression with auto-promotion

mod budget;
pub mod compression;
pub mod delta;
pub mod eviction;
pub mod prefetch;
mod types;
mod window;

pub use budget::{BudgetConfig, BudgetStatus, BudgetTracker, PressureLevel};
pub use compression::{CompressionPipeline, TierPromotion};
pub use eviction::{EvictionCandidate, EvictionRanker};
pub use prefetch::{
    CacheEntry, PrefetchCache, PrefetchEngine, PrefetchMetrics, PrefetchStrategy, TopicTracker,
};
pub use types::{
    CoherenceAlert, CompressionTier, DeliveredItem, EvictionPolicy, Session, SessionId,
};
pub use window::{WindowEstimator, WindowStatus};
