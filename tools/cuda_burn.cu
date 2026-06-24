#include <cuda_runtime.h>

#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>

namespace {

__global__ void burn_kernel(float* data, std::uint64_t iterations) {
    const std::uint64_t index = blockIdx.x * blockDim.x + threadIdx.x;
    float value = data[index];

    for (std::uint64_t iteration = 0; iteration < iterations; ++iteration) {
        value = fmaf(value, 1.000001f, 0.000001f);
        value = fmaf(value, 0.999999f, 0.000003f);
        value = __sinf(value) + __cosf(value);
    }

    data[index] = value;
}

void check(cudaError_t status, const char* operation) {
    if (status != cudaSuccess) {
        std::fprintf(stderr, "%s failed: %s\n", operation, cudaGetErrorString(status));
        std::exit(2);
    }
}

int parse_int(char** argv, int index, int fallback) {
    return argv[index] == nullptr ? fallback : std::atoi(argv[index]);
}

}  // namespace

int main(int argc, char** argv) {
    const int seconds = argc > 1 ? parse_int(argv, 1, 180) : 180;
    const int blocks = argc > 2 ? parse_int(argv, 2, 256) : 256;
    const int threads = argc > 3 ? parse_int(argv, 3, 256) : 256;
    const std::uint64_t iterations = argc > 4 ? std::strtoull(argv[4], nullptr, 10) : 20000ULL;

    const std::size_t elements = static_cast<std::size_t>(blocks) * static_cast<std::size_t>(threads);
    float* device_data = nullptr;
    check(cudaSetDevice(0), "cudaSetDevice");
    check(cudaMalloc(&device_data, elements * sizeof(float)), "cudaMalloc");
    check(cudaMemset(device_data, 1, elements * sizeof(float)), "cudaMemset");

    const auto started = std::chrono::steady_clock::now();
    int launches = 0;

    std::printf(
        "cuda_burn seconds=%d blocks=%d threads=%d iterations=%llu\n",
        seconds,
        blocks,
        threads,
        static_cast<unsigned long long>(iterations));
    std::fflush(stdout);

    while (std::chrono::steady_clock::now() - started < std::chrono::seconds(seconds)) {
        burn_kernel<<<blocks, threads>>>(device_data, iterations);
        check(cudaGetLastError(), "burn_kernel launch");
        check(cudaDeviceSynchronize(), "cudaDeviceSynchronize");
        ++launches;

        if (launches % 10 == 0) {
            const auto elapsed = std::chrono::duration_cast<std::chrono::seconds>(
                std::chrono::steady_clock::now() - started);
            std::printf("cuda_burn_progress launches=%d elapsed_s=%lld\n", launches, static_cast<long long>(elapsed.count()));
            std::fflush(stdout);
        }
    }

    check(cudaFree(device_data), "cudaFree");
    std::printf("cuda_burn_done launches=%d\n", launches);
    return 0;
}
