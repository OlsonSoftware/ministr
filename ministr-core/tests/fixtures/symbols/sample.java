package com.example;

/** Maximum retry count. */
public static final int MAX_RETRIES = 3;

/** A serialization interface. */
public interface Serializable {
    String serialize();
}

/** Configuration for the application. */
public class AppConfig implements Serializable {
    private String name;
    private boolean debug;

    public AppConfig(String name, boolean debug) {
        this.name = name;
        this.debug = debug;
    }

    /** Check if debug mode is on. */
    public boolean isDebug() {
        return this.debug;
    }

    @Override
    public String serialize() {
        return this.name;
    }
}

/** Greet a user by name. */
public static String greet(String name) {
    return "Hello, " + name + "!";
}

/** Color options. */
public enum Color {
    RED,
    GREEN,
    BLUE
}
