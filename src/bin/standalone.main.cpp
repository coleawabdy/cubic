import std;

int main()
{
    try
    {
        std::println("Hello, world!");
    }
    catch (const std::exception& e)
    {
        return 1;
    }
}
