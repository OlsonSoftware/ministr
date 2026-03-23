package com.example

/** Maximum retry count. */
const val MAX_RETRIES = 3

/** A serialization interface. */
interface Serializable {
    fun serialize(): String
}

/** Configuration for the application. */
class AppConfig(
    val name: String,
    val debug: Boolean = false
) : Serializable {

    /** Check if debug mode is on. */
    fun isDebug(): Boolean = debug

    override fun serialize(): String = name
}

/** Greet a user by name. */
fun greet(name: String): String {
    return "Hello, $name!"
}

/** Color options. */
enum class Color {
    RED,
    GREEN,
    BLUE
}
