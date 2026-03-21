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

mod budget;
pub mod delta;
mod types;
mod window;

pub use budget::{BudgetConfig, BudgetStatus, BudgetTracker, PressureLevel};
pub use types::{DeliveredItem, EvictionPolicy, Session, SessionId};
pub use window::{WindowEstimator, WindowStatus};
