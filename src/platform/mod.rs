pub mod fonts;
pub mod traits;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxBackend {
    Auto,
    Wayland,
    Evdev,
}

#[must_use]
pub fn parse_linux_backend(args: &[String]) -> LinuxBackend {
    if args
        .iter()
        .any(|a| a == "--backend=wayland" || a == "wayland")
    {
        LinuxBackend::Wayland
    } else if args.iter().any(|a| a == "--backend=evdev" || a == "evdev") {
        LinuxBackend::Evdev
    } else {
        LinuxBackend::Auto
    }
}

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;
