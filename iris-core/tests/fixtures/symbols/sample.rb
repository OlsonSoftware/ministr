# Maximum retry count.
MAX_RETRIES = 3

# Configuration for the application.
class AppConfig
  attr_accessor :name, :debug

  def initialize(name, debug: false)
    @name = name
    @debug = debug
  end

  # Check if debug mode is on.
  def debug?
    @debug
  end
end

# A module for serialization.
module Serializable
  def serialize
    to_s
  end
end

# Greet a user by name.
def greet(name)
  "Hello, #{name}!"
end
