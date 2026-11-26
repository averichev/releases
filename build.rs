use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publisher {
    product_id: String,
    public_key: PublicKey,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicKey {
    kty: String,
    crv: String,
    x: String,
}

fn main() {
    println!("cargo:rerun-if-env-changed=RELEASE_VERSION");
    let version = std::env::var("RELEASE_VERSION")
        .unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").expect("Cargo package version"));
    semver::Version::parse(&version).expect("RELEASE_VERSION must be a valid SemVer");
    println!("cargo:rustc-env=APP_VERSION={version}");
    println!(
        "cargo:rustc-env=APP_BUILD_TARGET={}",
        std::env::var("TARGET").expect("Cargo target")
    );
    println!("cargo:rerun-if-env-changed=RELEASE_UPDATE_IDENTITY");
    let path = std::env::var_os("RELEASE_UPDATE_IDENTITY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("deploy/self-update-identity.json"));
    println!("cargo:rerun-if-changed={}", path.display());
    let publisher: Publisher = serde_json::from_reader(
        std::fs::File::open(path).expect("Open the public release identity"),
    )
    .expect("Invalid public release identity");
    let product = publisher.product_id;
    assert!(
        uuid::Uuid::parse_str(&product).is_ok_and(|id| id.to_string() == product),
        "Release product_id must be a canonical UUID"
    );
    let key = publisher.public_key;
    assert!(
        key.kty == "OKP"
            && key.crv == "Ed25519"
            && URL_SAFE_NO_PAD
                .decode(&key.x)
                .is_ok_and(|bytes| bytes.len() == 32),
        "Expected a public Ed25519 JWK"
    );
    println!("cargo:rustc-env=APP_PRODUCT_ID={product}");
    println!("cargo:rustc-env=APP_UPDATE_PUBLIC_KEY={}", key.x);
    println!("cargo:rerun-if-changed=frontend/dist");
    println!("cargo:rerun-if-changed=migrations");
    assert!(
        std::path::Path::new("frontend/dist/index.html").is_file(),
        "Frontend build is missing. Run `npm --prefix frontend ci` and `npm --prefix frontend run build` before Cargo."
    );
}
