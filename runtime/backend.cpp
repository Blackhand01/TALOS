#include "runtime/backend.hpp"

#include "talos/ipc/cxx_bridge.rs.h"

namespace talos::runtime {

RuntimeResult run(rust::Slice<const std::uint8_t> buffer) {
    RuntimeResult result;
    result.ok = !buffer.empty();
    result.latency_ms = buffer.empty() ? 0 : 1 + static_cast<std::uint64_t>(buffer.size() / (1024 * 1024));
    return result;
}

}  // namespace talos::runtime
