"""Sample Python module for symbol extraction testing."""

MAX_RETRIES = 3


class AppConfig:
    """Configuration for the application."""

    def __init__(self, name: str, debug: bool = False):
        self.name = name
        self.debug = debug

    def is_debug(self) -> bool:
        """Check if debug mode is on."""
        return self.debug


def greet(name: str) -> str:
    """Greet a user by name."""
    return f"Hello, {name}!"


@property
def version():
    """Get the version string."""
    return "1.0.0"
