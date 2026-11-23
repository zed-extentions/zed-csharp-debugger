// Sample app for manually testing the netcoredbg Zed debugger extension.
// Suggested breakpoints are marked with [BP-n] comments.

using System.Diagnostics;

namespace DebugSample;

public record Person(string Name, int Age);

public static class Program
{
    public static int Main(string[] args)
    {
        // Pass "wait" to keep the process alive for the Attach scenario.
        bool waitMode = args.Contains("wait");

        Console.WriteLine($"debug-sample started (pid {Environment.ProcessId})");

        if (waitMode)
        {
            Console.WriteLine("Waiting for debugger... (attach now, press Enter to continue)");
            while (!Debugger.IsAttached && Console.In.Peek() == -1)
            {
                Thread.Sleep(200);
            }
        }

        // [BP-1] Stepping and local variables
        int a = 6;
        int b = 7;
        int product = Multiply(a, b);
        Console.WriteLine($"{a} * {b} = {product}");

        // [BP-2] Step into: recursion and call stack
        for (int n = 1; n <= 5; n++)
        {
            Console.WriteLine($"Factorial({n}) = {Factorial(n)}");
        }

        // [BP-3] Inspect collections and objects in the Variables panel
        var people = new List<Person>
        {
            new("Alice", 30),
            new("Bob", 25),
            new("Carol", 41),
        };
        var adults = people.Where(p => p.Age >= 30).Select(p => p.Name).ToList();
        Console.WriteLine($"Adults: {string.Join(", ", adults)}");

        // [BP-4] Conditional breakpoint: set condition `i == 3`
        int sum = 0;
        for (int i = 0; i < 10; i++)
        {
            sum += i;
        }
        Console.WriteLine($"sum = {sum}");

        // [BP-5] Exception handling: enable "break on exceptions" to stop at the throw
        try
        {
            Divide(10, 0);
        }
        catch (DivideByZeroException ex)
        {
            Console.WriteLine($"Caught: {ex.Message}");
        }

        // [BP-6] Multi-threading: check the Threads panel
        var worker = new Thread(() =>
        {
            Console.WriteLine($"Worker thread {Environment.CurrentManagedThreadId} running");
            Thread.Sleep(300);
        })
        { Name = "worker" };
        worker.Start();
        worker.Join();

        Console.WriteLine("Done.");
        return 0;
    }

    private static int Multiply(int x, int y)
    {
        int result = x * y; // [BP-1a] step into here
        return result;
    }

    private static long Factorial(int n)
    {
        if (n <= 1)
        {
            return 1;
        }
        return n * Factorial(n - 1);
    }

    private static int Divide(int x, int y)
    {
        return x / y; // throws DivideByZeroException when y == 0
    }
}
