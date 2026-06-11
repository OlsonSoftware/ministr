//! Operation handlers. Codes are assigned by the build from
//! `ops/registry.list`; handlers never know their own number.
pub mod beacon;
pub mod digest;
pub mod journal;
pub mod ledger;
pub mod mirror;
pub mod presence;
pub mod quota;
pub mod session;

