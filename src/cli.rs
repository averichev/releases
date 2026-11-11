use crate::{config::AppConfig, infrastructure::database};
use anyhow::{Context, Result, ensure};
use std::io::{self, BufRead, IsTerminal, Read, Write};
use zeroize::Zeroizing;
pub async fn bootstrap(config: &AppConfig, stdin: bool) -> Result<()> {
    // Bootstrap needs only SQLite, not artifact storage, a listener or signals.
    let pool = database::connect(&config.database).await?;
    let result = async {
        let state = crate::application_state(pool.clone(), config).await?;
        if state.authentication.admin_exists().await? {
            println!("Администратор уже существует. Новый пользователь не создан.");
            return Ok(());
        }
        let (username, password) =
            tokio::task::spawn_blocking(move || read_credentials(stdin)).await??;
        if state
            .authentication
            .bootstrap(&username, password.to_string())
            .await?
        {
            println!("Администратор создан. Войдите в /admin/login.");
        } else {
            println!("Администратор уже существует. Новый пользователь не создан.");
        }
        Ok(())
    }
    .await;
    pool.close().await;
    result
}
/// Owner-operated recovery works even when the sole administrator lost access.
pub async fn reset_password(config: &AppConfig, stdin: bool) -> Result<()> {
    let pool = database::connect(&config.database).await?;
    let result = async {
        let state = crate::application_state(pool.clone(), config).await?;
        let username = tokio::task::spawn_blocking(move || read_username(stdin)).await??;
        let issued = state
            .users
            .recover_administrator_from_console(&username)
            .await?;
        println!(
            "Одноразовая ссылка восстановления (срок {} секунд):",
            config.auth.invitation_ttl_seconds
        );
        println!(
            "{}/admin/activate#token={}",
            config.auth.public_origin, issued.token
        );
        println!("Предыдущая ссылка отменена. Сохраните эту ссылку в безопасном месте.");
        Ok(())
    }
    .await;
    pool.close().await;
    result
}
fn read_username(stdin: bool) -> Result<String> {
    if stdin {
        ensure!(!io::stdin().is_terminal(), "--stdin requires a pipe");
    } else {
        ensure!(
            io::stdin().is_terminal(),
            "Use --stdin explicitly for automated input"
        );
        print!("Логин администратора: ");
        io::stdout().flush()?;
    }
    Ok(line(&mut io::stdin().lock())?.to_string())
}
fn line(reader: &mut impl BufRead) -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new(Vec::new());
    (&mut *reader).take(1026).read_until(b'\n', &mut bytes)?;
    ensure!(
        bytes.len() <= 1025 && bytes.ends_with(b"\n"),
        "Expected a line of at most 1024 bytes"
    );
    let value = Zeroizing::new(String::from_utf8(bytes.to_vec()).context("Input must be UTF-8")?);
    Ok(Zeroizing::new(
        value.trim_end_matches(['\r', '\n']).to_owned(),
    ))
}
fn read_credentials(stdin: bool) -> Result<(String, Zeroizing<String>)> {
    if stdin {
        ensure!(
            !io::stdin().is_terminal(),
            "--stdin requires a pipe: username and password on separate lines"
        );
        let mut reader = io::stdin().lock();
        let username = line(&mut reader)?;
        let password = line(&mut reader)?;
        return Ok((username.to_string(), password));
    }
    ensure!(
        io::stdin().is_terminal(),
        "Use --stdin explicitly for automated input"
    );
    print!("Логин (3–64 символа: a-z, 0-9, ._-): ");
    io::stdout().flush()?;
    let username = line(&mut io::stdin().lock())?;
    let password = Zeroizing::new(rpassword::prompt_password("Пароль (12–128 символов): ")?);
    let confirmation = Zeroizing::new(rpassword::prompt_password("Повторите пароль: ")?);
    ensure!(*password == *confirmation, "Passwords do not match");
    Ok((username.to_string(), password))
}
