// Embeds assets/icon.ico as the Windows .exe icon (shown in Explorer, taskbar, Alt+Tab).
// A failure here only costs the file icon, so it warns instead of breaking the build.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the exe icon: {e}");
        }
    }
}
