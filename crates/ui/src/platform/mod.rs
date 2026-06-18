pub mod fonts;

#[cfg(target_os = "linux")]
pub fn setup_autostart() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME").ok_or("HOME directory not set")?;
    let autostart_dir = std::path::Path::new(&home).join(".config").join("autostart");
    std::fs::create_dir_all(&autostart_dir)?;

    let exec_path = std::env::current_exe()?;
    let desktop_path = autostart_dir.join("qianyan-ime.desktop");
    let content = format!(
        "[Desktop Entry]\n\
         Name=Qianyan-IME\n\
         Comment=千言输入法开机自启\n\
         Exec={}\n\
         Icon=qianyan-ime\n\
         Terminal=false\n\
         Type=Application\n\
         Categories=Settings;Utility;\n\
         StartupNotify=false\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_path.display()
    );
    std::fs::write(&desktop_path, content)?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn remove_autostart() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME").ok_or("HOME directory not set")?;
    let desktop_path = std::path::Path::new(&home)
        .join(".config")
        .join("autostart")
        .join("qianyan-ime.desktop");
    if desktop_path.exists() {
        std::fs::remove_file(&desktop_path)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn setup_autostart() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let exe_path = exe.to_str().ok_or("Invalid path")?;
    let _ = std::process::Command::new("reg")
        .arg("add")
        .arg("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .arg("/v")
        .arg("QianyanIME")
        .arg("/t")
        .arg("REG_SZ")
        .arg("/d")
        .arg(exe_path)
        .arg("/f")
        .status();
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn remove_autostart() -> Result<(), Box<dyn std::error::Error>> {
    let _ = std::process::Command::new("reg")
        .arg("delete")
        .arg("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .arg("/v")
        .arg("QianyanIME")
        .arg("/f")
        .status();
    Ok(())
}
