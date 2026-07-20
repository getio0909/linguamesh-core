#include <linguamesh/linguamesh.hpp>

#include <cassert>
#include <chrono>
#include <cstdint>
#include <iostream>
#include <utility>
#include <vector>

int main() {
    static_assert(
        static_cast<std::int32_t>(linguamesh::result_code::resource_exhausted)
        == LM_RESULT_RESOURCE_EXHAUSTED);
    const auto compatibility = linguamesh::engine::query_compatibility();
    assert(compatibility.abi_major == LM_ABI_VERSION_MAJOR);
    assert(compatibility.protocol == LM_PROTOCOL_VERSION);

    bool rejected = false;
    try {
        static_cast<void>(linguamesh::engine::create(LM_ABI_VERSION_MAJOR + 1));
    } catch (const linguamesh::compatibility_error& error) {
        rejected = error.actual_abi() == LM_ABI_VERSION_MAJOR;
    }
    assert(rejected);

    auto engine = linguamesh::engine::create();
    const std::vector<std::uint8_t> path{'/', 't', 'm', 'p', '/', 'l', 'i', 'n', 'g', 'u', 'a',
                                         'm', 'e', 's', 'h', '-', 'i', 'n', 'p', 'u', 't'};
    auto lease = engine.create_desktop_path(path);
    assert(lease.valid());
    assert(lease.is_active());
    lease.expire();
    assert(!lease.is_active());
    lease.reset();
    auto descriptor_lease = engine.create_posix_descriptor(0);
    assert(descriptor_lease.is_active());
    descriptor_lease.revoke();
    assert(!descriptor_lease.is_active());
    descriptor_lease.reset();
    const std::vector<std::uint8_t> malformed{0xff};
    try {
        engine.submit(malformed);
        assert(false);
    } catch (const linguamesh::core_error& error) {
        assert(error.code() == linguamesh::result_code::malformed_message);
    }

    auto event = engine.poll_event(std::chrono::milliseconds(0));
    assert(event.empty());
    auto moved = std::move(engine);
    assert(!engine.is_open());
    assert(moved.is_open());
    moved.cancel();
    moved.shutdown();
    std::cout << "C++ wrapper smoke test passed.\n";
    return 0;
}
