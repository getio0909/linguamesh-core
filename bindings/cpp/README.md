# C++ Native Wrapper

`include/linguamesh/linguamesh.hpp` is a header-only C++20 RAII layer over
`contracts/abi/linguamesh.h`. It owns `LmEngine`, copies and releases each engine-owned `LmBuffer`
before returning an event vector, converts stable result codes to typed exceptions, validates ABI
and protocol versions before engine creation, and exposes bounded event polling suitable for a
dedicated C++/WinRT worker thread. It also exposes move-only `file_lease` values for the bounded
ABI lifecycle controls; lease tokens are engine-scoped and never expose platform resource values.
Destroying an engine invalidates outstanding wrapper leases without calling through a stale handle.

Add both include directories and link the Rust library:

```cmake
target_compile_features(app PRIVATE cxx_std_20)
target_include_directories(app PRIVATE
    path/to/linguamesh-core/contracts/abi
    path/to/linguamesh-core/bindings/cpp/include)
target_link_libraries(app PRIVATE linguamesh_ffi)
```

Do not poll on the WinUI dispatcher thread. Encode commands and decode events using the checked-in
`contracts/proto/linguamesh.proto` schema. This is a generic C++20 wrapper, not a generated
C++/WinRT projection. A C++/WinRT client may isolate it behind its native bridge. `engine` is
movable but not copyable; keep one owner and destroy it only after polling work has stopped. A
future Windows prerelease package will place this header, `linguamesh.h`, the DLL/import library,
symbols, metadata, and checksums together.
