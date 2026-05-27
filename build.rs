fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("android") {
        // 获取 NDK 库路径（不要用 cargo:rustc-link-search 指向该目录，
        // 因为其中还有 libc.a 会导致链接器优先静态链接 libc，留下未定义符号）
        let ndk_lib_path: String = std::env::var("CARGO_NDK_SYSROOT_LIBS_PATH")
            .or_else(|_| -> Result<String, std::env::VarError> {
                let ndk_home =
                    std::env::var("ANDROID_NDK_HOME").or_else(|_| std::env::var("NDK_HOME"))?;
                let arch_lib = target.replace('-', "-");
                Ok(format!(
                    "{}/toolchains/llvm/prebuilt/windows-x86_64/sysroot/usr/lib/{}",
                    ndk_home, arch_lib,
                ))
            })
            .expect("无法找到 NDK 库路径");

        // 直接用完整路径传递 .a 文件，避免 -L 引入 libc.a 导致静态链接 libc
        let libcxx_static = format!("{}/libc++_static.a", ndk_lib_path);
        let libcxxabi = format!("{}/libc++abi.a", ndk_lib_path);

        println!("cargo:rustc-link-arg=-Wl,--start-group");
        println!("cargo:rustc-link-arg={}", libcxx_static);
        println!("cargo:rustc-link-arg={}", libcxxabi);
        println!("cargo:rustc-link-arg=-Wl,--end-group");
    }
}
