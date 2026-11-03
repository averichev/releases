fn main() {
    println!("cargo:rerun-if-changed=frontend/dist");
    println!("cargo:rerun-if-changed=migrations");
    assert!(
        std::path::Path::new("frontend/dist/index.html").is_file(),
        "Frontend build is missing. Run `npm --prefix frontend ci` and `npm --prefix frontend run build` before Cargo."
    );
}
