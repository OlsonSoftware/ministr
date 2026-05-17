//! Session shadow and budget management for ministr.
//!
//! This module implements the session intelligence layer — tracking what content
//! has been delivered to an agent, estimating context window usage, and managing
//! token budgets with pressure-based response compression.
//!
//! # Architecture
//!
//! - [`Session`] — tracks delivered content, access trajectory, and dedup state
//! - [`WindowEstimator`] — models context window fill with FIFO/LRU eviction
//! - [`UsageTracker`] — threshold-based pressure levels driving response compression
//! - [`DropRanker`] — scores delivered items for eviction priority
//! - [`CompressionPipeline`] — multi-tier compression with auto-promotion

pub mod compression;
pub mod delta;
pub mod drops;
pub mod memory;
pub mod prefetch;
mod registry;
mod types;
mod usage;
mod window;

pub use compression::{CompressionPipeline, TierPromotion};
pub use drops::{DropCandidate, DropRanker};
pub use memory::{AccessRating, MemoryState, MemoryTracker};
pub use prefetch::{
    CacheEntry, PrefetchEngine, PrefetchMetrics, PrefetchStrategy, PriorityCache, StrategyWeights,
    TopicTracker,
};
pub use registry::{SessionEntry, SessionRegistry};
pub use types::{
    AccessMode, CoherenceAlert, CompressionTier, DeliveredItem, DropPolicy, Session, SessionId,
    SessionMetrics,
};
pub use usage::{UsageConfig, UsageLevel, UsageStatus, UsageTracker};
pub use window::{WindowEstimator, WindowStatus};
