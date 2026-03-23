/// Maximum retry count.
let MAX_RETRIES = 3

/// A protocol for serialization.
protocol Serializable {
    func serialize() -> String
}

/// Configuration for the application.
public class AppConfig: Serializable {
    var name: String
    var debug: Bool

    init(name: String, debug: Bool = false) {
        self.name = name
        self.debug = debug
    }

    /// Check if debug mode is on.
    func isDebug() -> Bool {
        return debug
    }

    func serialize() -> String {
        return name
    }
}

/// Greet a user by name.
public func greet(name: String) -> String {
    return "Hello, \(name)!"
}

/// Color options.
enum Color {
    case red
    case green
    case blue
}
