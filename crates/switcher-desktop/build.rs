fn main() {
    println!("cargo:rerun-if-changed=assets/codex-auth-switcher.rc");
    println!("cargo:rerun-if-changed=assets/codex-auth-switcher.ico");

    #[cfg(windows)]
    embed_resource::compile("assets/codex-auth-switcher.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap_or_else(|error| panic!("cannot embed the Windows application icon: {error}"));
}
