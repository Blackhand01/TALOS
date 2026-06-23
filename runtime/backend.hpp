#pragma once

#include <cstdint>

#include "rust/cxx.h"

namespace talos::runtime {

struct RuntimeResult;

RuntimeResult run(rust::Slice<const std::uint8_t> buffer);
RuntimeResult run_cv_features(rust::Slice<const std::uint8_t> buffer);

}  // namespace talos::runtime
