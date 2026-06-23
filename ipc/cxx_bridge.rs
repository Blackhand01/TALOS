#[cxx::bridge(namespace = "talos::runtime")]
pub mod ffi {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct RuntimeResult {
        pub ok: bool,
        pub latency_ms: u64,
        pub feature_dim: u32,
        pub input_bytes: u64,
        pub mean: f32,
        pub variance: f32,
        pub min_value: f32,
        pub max_value: f32,
        pub edge_density: f32,
        pub entropy: f32,
        pub checksum: u64,
    }

    unsafe extern "C++" {
        include!("runtime/backend.hpp");

        fn run(buffer: &[u8]) -> RuntimeResult;
        fn run_cv_features(buffer: &[u8]) -> RuntimeResult;
    }
}
