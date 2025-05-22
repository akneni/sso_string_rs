# SsoString: Optimized Rust Strings

`SsoString` is a Rust string type engineered for performance, featuring **Small String Optimization (SSO)** and **Copy-on-Write (CoW)** capabilities. It's designed to be a more efficient alternative in scenarios involving numerous small strings or frequent use of static string data.

## Core Optimizations:

* **Small String Optimization (SSO):** Say goodbye to unnecessary heap allocations! Strings up to 23 bytes are stored directly inline within the `SsoString` structure itself. This can significantly speed up operations when you're dealing with lots of small text snippets.
* **Copy-on-Write (CoW) for Static Data:** You can create an `SsoString` from static string literals (`&'static str`) using `SsoString::from_static()`. These strings initially just point to the static data, making their creation and cloning lightning-fast (essentially just a pointer copy). The actual string data is only copied to a new heap allocation if, and when, the string needs to be modified.
* **Heap Allocation for Larger Strings:** When strings grow beyond the inline capacity, `SsoString` seamlessly transitions to allocating memory on the heap, much like the standard `std::string::String`.
* **Familiar API (Work in Progress):** We're working towards an API that's largely compatible with `std::string::String`. The goal is to make it easy to integrate `SsoString` into your projects and use it with a familiar set of operations.

## Benchmarks (WIP)
```
all  < 23 characters
=================================
::from(&str) | 1000000 strings | 
SsoString:    21.160817ms
compact_str:  21.080907ms
String:       48.826193ms


::push_str("abc") | 1000000 base strings | 
SsoString:    11.646899ms
compact_str:  10.899839ms
String:       41.096044ms


::push_str("01234567890123456789") | 1000000 base strings | 
SsoString:    29.966386ms
compact_str:  42.713364ms
String:       47.186134ms


::cmp(&str) | 1000000 strings
SsoString:    6.690129ms
compact_str:  7.546749ms
String:       4.558759ms
=================================


half < 23 characters 
=================================
::from(&str) | 1000000 strings | 
SsoString:    24.815847ms
compact_str:  16.830967ms
String:       20.684017ms


::push_str("abc") | 1000000 base strings | 
SsoString:    42.415844ms
compact_str:  46.862964ms
String:       69.74364ms


::push_str("01234567890123456789") | 1000000 strings | 
SsoString:    32.366086ms
compact_str:  43.996394ms
String:       61.504891ms


::cmp(&str) | 1000000 strings
SsoString:    9.558229ms
compact_str:  9.321669ms
String:       5.672679ms
=================================


none < 23 characters
=================================
::from(&str) | 1000000 strings |
SsoString:    17.054118ms
compact_str:  17.314587ms
String:       16.995058ms


::push_str("abc") | 1000000 base strings |
SsoString:    32.535416ms
compact_str:  51.312873ms
String:       97.145326ms


::push_str("01234567890123456789") | 1000000 strings |
SsoString:    31.682835ms
compact_str:  74.695649ms
String:       85.518098ms


::cmp(&str) | 1000000 strings
SsoString:    7.707849ms
compact_str:  8.986779ms
String:       5.077099ms
=================================
```