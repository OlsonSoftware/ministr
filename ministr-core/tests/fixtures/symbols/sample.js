/**
 * Maximum retry count.
 */
const MAX_RETRIES = 3;

/**
 * Configuration for the application.
 */
class AppConfig {
    constructor(name, debug = false) {
        this.name = name;
        this.debug = debug;
    }

    isDebug() {
        return this.debug;
    }
}

/**
 * Greet a user by name.
 */
function greet(name) {
    return `Hello, ${name}!`;
}

export default AppConfig;
