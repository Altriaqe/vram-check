//! 把 `app.manifest` 和 `app.rc`（图标 + 版本信息）嵌进 exe。
//!
//! 不引第三方构建依赖：
//! - manifest 走 MSVC link.exe 的 `/MANIFESTINPUT`；
//! - 图标/版本资源用 Windows SDK 的 `rc.exe` 编译成 `.res`，再交给链接器。
//!
//! 找不到 rc.exe 时只跳过资源（给出 warning），不让构建直接失败。

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

    let manifest = manifest_dir.join("app.manifest");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );

    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=app.ico");
    match find_rc() {
        Some(rc) => {
            let resource = out_dir.join("app.res");
            let status = Command::new(&rc)
                .current_dir(&manifest_dir)
                .arg("/nologo")
                .arg("/fo")
                .arg(&resource)
                .arg("app.rc")
                .status();
            match status {
                Ok(status) if status.success() && resource.exists() => {
                    println!("cargo:rustc-link-arg-bins={}", resource.display());
                }
                _ => println!("cargo:warning=rc.exe 执行失败，跳过图标和版本资源"),
            }
        }
        None => println!("cargo:warning=找不到 rc.exe，跳过图标和版本资源"),
    }
}

fn find_rc() -> Option<PathBuf> {
    if let Ok(sdk) = std::env::var("WindowsSdkVerBinPath") {
        let candidate = Path::new(&sdk).join("x64").join("rc.exe");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let kits = Path::new(&std::env::var("ProgramFiles(x86)").ok()?)
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&kits)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();
    for version in versions.iter().rev() {
        let candidate = version.join("x64").join("rc.exe");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}
