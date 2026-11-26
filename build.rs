use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Publisher {
    Release {
        product_id: String,
        updates: Updates,
    },
    // Existing RELEASE_UPDATE_IDENTITY files store the JWK at the top level.
    Legacy {
        product_id: String,
        public_key: PublicKey,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Updates {
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
    println!("cargo:rerun-if-env-changed=VERUNA_RELEASE_VERSION");
    println!("cargo:rerun-if-env-changed=RELEASE_VERSION");
    println!("cargo:rerun-if-env-changed=RELEASE_UPDATE_IDENTITY");
    let identity = PathBuf::from("deploy/self-update-identity.json");
    let (version, path) = match std::env::var("VERUNA_RELEASE_VERSION") {
        // VerunaPack signs against the identity committed in its Git snapshot.
        Ok(version) => (version, identity),
        Err(_) => (
            std::env::var("RELEASE_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into()),
            std::env::var_os("RELEASE_UPDATE_IDENTITY")
                .map(PathBuf::from)
                .unwrap_or(identity),
        ),
    };
    semver::Version::parse(&version).expect("Release version must be a valid SemVer");
    println!("cargo:rustc-env=APP_VERSION={version}");
    println!(
        "cargo:rustc-env=APP_BUILD_TARGET={}",
        std::env::var("TARGET").expect("Cargo target")
    );
    println!("cargo:rerun-if-changed={}", path.display());
    let publisher: Publisher = serde_json::from_reader(
        std::fs::File::open(path).expect("Open the public release identity"),
    )
    .expect("Invalid public release identity");
    let (product, key) = match publisher {
        Publisher::Release {
            product_id,
            updates,
        } => (product_id, updates.public_key),
        Publisher::Legacy {
            product_id,
            public_key,
        } => (product_id, public_key),
    };
    assert!(
        uuid::Uuid::parse_str(&product).is_ok_and(|id| id.to_string() == product),
        "Release product_id must be a canonical UUID"
    );
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
