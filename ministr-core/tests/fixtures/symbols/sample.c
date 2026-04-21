/**
 * Maximum retry count.
 */
#define MAX_RETRIES 3

/**
 * Application configuration.
 */
struct AppConfig {
    char *name;
    int debug;
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
void greet(const char *name) {
    printf("Hello, %s!\n", name);
}

/**
 * Create a new AppConfig.
 */
struct AppConfig new_config(const char *name) {
    struct AppConfig cfg;
    cfg.name = (char *)name;
    cfg.debug = 0;
    return cfg;
}
