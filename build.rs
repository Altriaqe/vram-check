//! 把 `app.manifest` 嵌进 exe：comctl32 v6 + PerMonitorV2 DPI。
//!
//! 不引第三方构建依赖：直接用 MSVC link.exe 的 `/MANIFESTINPUT` 参数。
//! 如果换用 lld 链接器，这两个参数需要改成 lld 的等价写法。

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("app.manifest");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
