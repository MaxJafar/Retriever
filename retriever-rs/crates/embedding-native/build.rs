fn main() {
    println!("cargo:rerun-if-env-changed=RETRIEVER_EMBEDDING_CUDA");
    println!("cargo:rerun-if-env-changed=RETRIEVER_CUDA_ARCHITECTURES");
    let cuda = std::env::var("RETRIEVER_EMBEDDING_CUDA").as_deref() == Ok("1");
    let source = std::path::Path::new("../../vendor/llama.cpp");
    println!("cargo:rerun-if-changed=bridge.cpp");
    println!("cargo:rerun-if-changed={}", source.display());
    let mut build = cmake::Config::new(source);
    // Optimized, portable CPU kernels even when the Rust caller is a debug build.
    build.profile("Release");
    // cmake-rs rewrites MSVC's configuration flags to communicate /MD, dropping
    // CMake's /O2 and NDEBUG defaults. Explicitly retain optimization: otherwise
    // a Cargo release build silently runs unoptimized ggml tensor kernels.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build.define("CMAKE_C_FLAGS_RELEASE", "/O2 /Ob2 /DNDEBUG /MD");
        build.define("CMAKE_CXX_FLAGS_RELEASE", "/O2 /Ob2 /DNDEBUG /MD");
    }
    for option in [
        "BUILD_SHARED_LIBS",
        "LLAMA_BUILD_COMMON",
        "LLAMA_BUILD_TESTS",
        "LLAMA_BUILD_TOOLS",
        "LLAMA_BUILD_EXAMPLES",
        "LLAMA_BUILD_APP",
        "LLAMA_BUILD_MTMD",
        "GGML_NATIVE",
        "GGML_OPENMP",
        "GGML_METAL",
        "GGML_BLAS",
        "GGML_LLAMAFILE",
    ] {
        build.define(option, "OFF");
    }
    build.define("LLAMA_BUILD_NUMBER", "0");
    build.define("GGML_CUDA", if cuda { "ON" } else { "OFF" });
    if cuda {
        build.define("GGML_CUDA_NO_VMM", "ON");
        build.define("GGML_CUDA_FA", "OFF");
        if let Ok(architectures) = std::env::var("RETRIEVER_CUDA_ARCHITECTURES") {
            build.define("CMAKE_CUDA_ARCHITECTURES", architectures);
        }
    }
    build.define(
        "LLAMA_BUILD_COMMIT",
        "4d756bc72bf00a4aacf410ae15a2d315f3db400d",
    );
    let dst = build.build();
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .opt_level(2)
        .include(source.join("include"))
        .include(source.join("ggml/include"))
        .file("bridge.cpp")
        .compile("retriever_embedding_bridge");
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-search=native={}/lib64", dst.display());
    for library in ["llama", "ggml", "ggml-cpu", "ggml-base"] {
        println!("cargo:rustc-link-lib=static={library}");
    }
    if cuda {
        println!("cargo:rustc-link-lib=static=ggml-cuda");
        let toolkit = std::env::var("CUDA_PATH").expect("CUDA_PATH is required for a CUDA build");
        println!("cargo:rustc-link-search=native={toolkit}/lib/x64");
        for library in ["cudart", "cublas", "cublasLt"] {
            println!("cargo:rustc-link-lib={library}");
        }
    }
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target == "macos" {
        println!("cargo:rustc-link-lib=framework=Accelerate");
    }
    if target == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
