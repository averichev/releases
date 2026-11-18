fn main() {
    println!("cargo:rerun-if-env-changed=RELEASE_VERSION");
    let version = std::env::var("RELEASE_VERSION")
        .unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").expect("Cargo package version"));
    semver::Version::parse(&version).expect("RELEASE_VERSION must be a valid SemVer");
    println!("cargo:rustc-env=APP_VERSION={version}");
    println!("cargo:rerun-if-changed=frontend/dist");
    println!("cargo:rerun-if-changed=migrations");
    assert!(
        std::path::Path::new("frontend/dist/index.html").is_file(),
        "Frontend build is missing. Run `npm --prefix frontend ci` and `npm --prefix frontend run build` before Cargo."
    );
}
