namespace Sample;

/// <summary>Maximum retry count.</summary>
public const int MaxRetries = 3;

/// <summary>A serialization interface.</summary>
public interface ISerializable
{
    string Serialize();
}

/// <summary>Configuration for the application.</summary>
public class AppConfig : ISerializable
{
    public string Name { get; set; }
    public bool Debug { get; set; }

    public AppConfig(string name, bool debug)
    {
        Name = name;
        Debug = debug;
    }

    /// <summary>Check if debug mode is on.</summary>
    public bool IsDebug()
    {
        return Debug;
    }

    public string Serialize()
    {
        return Name;
    }
}

/// <summary>Color options.</summary>
public enum Color
{
    Red,
    Green,
    Blue
}

/// <summary>Greet a user by name.</summary>
public static string Greet(string name)
{
    return $"Hello, {name}!";
}
