# SsoString: Optimized Rust Strings

`SsoString` is a Rust string type engineered for performance, featuring **Small String Optimization (SSO)** and **Copy-on-Write (CoW)** capabilities. It's designed to be a more efficient alternative in scenarios involving numerous small strings or frequent use of static string data.

## Core Optimizations:

* **Small String Optimization (SSO):** Say goodbye to unnecessary heap allocations! Strings up to 23 bytes are stored directly inline within the `SsoString` structure itself. This can significantly speed up operations when you're dealing with lots of small text snippets.
* **Copy-on-Write (CoW) for Static Data:** You can create an `SsoString` from static string literals (`&'static str`) using `SsoString::from_static()`. These strings initially just point to the static data, making their creation and cloning lightning-fast (essentially just a pointer copy). The actual string data is only copied to a new heap allocation if, and when, the string needs to be modified.
* **Heap Allocation for Larger Strings:** When strings grow beyond the inline capacity, `SsoString` seamlessly transitions to allocating memory on the heap, much like the standard `std::string::String`.
* **Familiar API (Work in Progress):** We're working towards an API that's largely compatible with `std::string::String`. The goal is to make it easy to integrate `SsoString` into your projects and use it with a familiar set of operations.

If your application handles many small strings or works extensively with static text, `SsoString` could offer a noticeable performance boost by reducing memory allocation overhead and minimizing data copying.