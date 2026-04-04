use std::env;
use std::path::PathBuf;

fn main() {
    slint_build::compile("src/ui/main.slint").expect("Failed to compile Slint UI");

    // 为 Wayland 协议生成 Rust 绑定
    if cfg!(target_os = "linux") {
        let protocols_dir =
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"))
                .join("protocols");

        // 生成 virtual-keyboard-v1
        let vk_xml = protocols_dir.join("virtual-keyboard-v1.xml");
        if vk_xml.exists() {
            println!("cargo:rerun-if-changed=protocols/virtual-keyboard-v1.xml");
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("picture/rust-ime.ico");
        res.compile().expect("Failed to compile Windows resources");
    }
}
