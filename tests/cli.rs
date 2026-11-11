use std::{
    io::Write,
    process::{Command, Stdio},
};
use tempfile::tempdir;
#[tokio::test]
async fn bootstrap_and_owner_recovery_are_independent_and_do_not_echo_passwords() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("db.sqlite");
    let artifacts = directory.path().join("not-a-directory");
    std::fs::write(&artifacts, "existing").unwrap();
    let command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_veruna-releases"));
        command
            .env_clear()
            .env("VERUNA__DATABASE__PATH", &database)
            .env("VERUNA__STORAGE__DIRECTORY", &artifacts)
            .current_dir(directory.path());
        command
    };
    let mut child = command()
        .args(["admin", "bootstrap", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"owner\nsecret-password-123\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("secret-password-123"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-password-123"));
    // The second invocation succeeds without reading stdin or opening HTTP/storage.
    let output = command()
        .args(["admin", "bootstrap"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("уже существует"));
    let output = command()
        .args(["admin", "bootstrap", "secret-password-123"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-password-123"));
    let mut links = Vec::new();
    for _ in 0..2 {
        let mut child = command()
            .args(["admin", "reset-password", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"owner\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        let url = stdout.lines().find(|s| s.starts_with("http://")).unwrap();
        let token = url.split("#token=").nth(1).unwrap().to_owned();
        assert!(!String::from_utf8_lossy(&output.stderr).contains(&token));
        links.push(token);
    }
    let mut config = veruna_releases::config::AppConfig::default();
    config.database.path = database;
    let pool = veruna_releases::infrastructure::database::connect(&config.database)
        .await
        .unwrap();
    let state = veruna_releases::application_state(pool.clone(), &config)
        .await
        .unwrap();
    assert!(state.authentication.inspect_token(&links[0]).await.is_err());
    state
        .authentication
        .activate(&links[1], "recovered-password-456".into())
        .await
        .unwrap();
    assert!(
        state
            .authentication
            .activate(&links[1], "another-password-789".into())
            .await
            .is_err()
    );
    assert!(
        state
            .authentication
            .login("owner", "secret-password-123".into(), None)
            .await
            .is_err()
    );
    state
        .authentication
        .login("owner", "recovered-password-456".into(), None)
        .await
        .unwrap();
    assert!(
        !state
            .authentication
            .bootstrap("extra", "new-password-456".into())
            .await
            .unwrap()
    );
    pool.close().await;
}
