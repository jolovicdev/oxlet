fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icons/oxlet.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon_with_id("../../packaging/icons/oxlet.ico", "1");
    res.set("ProductName", "Oxlet");
    res.set("FileDescription", "Oxlet");
    if let Err(e) = res.compile() {
        // MSVC always has the resource compiler; elsewhere a missing windres only costs the icon.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            panic!("embedding the Windows icon: {e}");
        }
        println!("cargo:warning=Oxlet.exe built without its icon: {e}");
    }
}
