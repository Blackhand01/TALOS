fn main() {
    cxx_build::bridge("ipc/cxx_bridge.rs")
        .file("runtime/backend.cpp")
        .file("runtime/cv/features.cpp")
        .flag_if_supported("-std=c++20")
        .include(".")
        .compile("talos_runtime");

    println!("cargo:rerun-if-changed=ipc/cxx_bridge.rs");
    println!("cargo:rerun-if-changed=runtime/backend.cpp");
    println!("cargo:rerun-if-changed=runtime/backend.hpp");
    println!("cargo:rerun-if-changed=runtime/cv/features.cpp");
    println!("cargo:rerun-if-changed=runtime/cv/features.hpp");
}
