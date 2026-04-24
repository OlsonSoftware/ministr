package sample

// MaxRetries is the maximum retry count.
const MaxRetries = 3

// AppConfig holds application configuration.
type AppConfig struct {
	Name  string
	Debug bool
}

// Serializable defines a serialization interface.
type Serializable interface {
	Serialize() string
}

// Greet returns a greeting for the given name.
func Greet(name string) string {
	return "Hello, " + name + "!"
}

// NewAppConfig creates a new AppConfig with defaults.
func NewAppConfig() *AppConfig {
	return &AppConfig{
		Name:  "",
		Debug: false,
	}
}
