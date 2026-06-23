#[cxx::bridge(namespace = "talos::runtime")]
pub mod ffi {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct RuntimeResult {
        pub ok: bool,
        pub latency_ms: u64,
    }

    unsafe extern "C++" {
        include!("runtime/backend.hpp");

        fn run(buffer: &[u8]) -> RuntimeResult;
    }
}
