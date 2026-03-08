fn main() {
    // Embed a Windows application icon into the executable.
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/strikehub.ico");
        res.set("ProductName", "StrikeHub");
        res.set("FileDescription", "StrikeHub - Strike48 Connector Hub");
        res.compile().expect("failed to compile Windows resources");
    }
}
