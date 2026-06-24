#pragma once

#include <cstdint>

#include "rust/cxx.h"

namespace talos::runtime::vlm {

struct QuerySummary {
    bool ok;
    std::uint64_t input_bytes;
    std::uint64_t checksum;
    std::uint32_t output_tokens;
    float confidence;
    std::uint32_t answer_code;
};

QuerySummary run_quantized_query(rust::Slice<const std::uint8_t> buffer);

}  // namespace talos::runtime::vlm
