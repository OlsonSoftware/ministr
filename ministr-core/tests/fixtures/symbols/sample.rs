/// Configuration for the application.
pub struct AppConfig {
    pub name: String,
    pub debug: bool,
}

/// Color options.
pub enum Color {
    Red,
    Green,
    Blue,
}

/// A trait for serialization.
pub trait Serialize {
    fn serialize(&self) -> String;
}

/// Maximum retry count.
pub const MAX_RETRIES: u32 = 3;

/// Greet a user by name.
pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

impl AppConfig {
    /// Create a new config with defaults.
    pub fn new() -> Self {
        Self {
            name: String::new(),
            debug: false,
        }
    }
}
