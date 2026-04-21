/**
 * Maximum retry count.
 */
const int MAX_RETRIES = 3;

/**
 * Application configuration.
 */
class AppConfig {
public:
    std::string name;
    bool debug;

    AppConfig(std::string name, bool debug)
        : name(std::move(name)), debug(debug) {}

    bool isDebug() const {
        return debug;
    }
};

/**
 * Color options.
 */
enum Color {
    RED,
    GREEN,
    BLUE
};

/**
 * Greet a user by name.
 */
void greet(const std::string &name) {
    std::cout << "Hello, " << name << "!" << std::endl;
}
