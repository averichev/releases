import client from '../../../docs/integrations/tauri/client.ts?raw';
import button from '../../../docs/integrations/tauri/button.ts?raw';
export { client, button };
export type Manager = 'npm' | 'pnpm' | 'yarn' | 'bun';
export function command(manager: Manager, action: string) {
  const runner = { npm: 'npm run tauri', pnpm: 'pnpm tauri', yarn: 'yarn tauri', bun: 'bunx tauri' }[manager];
  return `${runner} ${action}`;
}
export function signer(manager: Manager, slug: string) {
  return command(manager, `signer generate ${manager === 'npm' ? '-- ' : ''}-w ~/.tauri/${slug}.key`);
}
export const register = `// В существующем .setup(|app| { ... }) добавьте:
#[cfg(desktop)]
{
    app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
    app.handle().plugin(tauri_plugin_process::init())?;
}
// Сохраните остальной setup и завершающий Ok(()).
// Если setup ещё нет, добавьте .setup(|app| { ...; Ok(()) })
// в цепочку tauri::Builder::default() перед .run(...).`;
export const capabilities = JSON.stringify({ permissions: ['updater:default', 'process:allow-restart'] }, null, 2);
export function runtime(endpoints: {stable: string; beta: string}) {
  return `use tauri_plugin_updater::UpdaterExt;

// Вызывайте из Rust, передав выбор пользователя из настроек клиента.
#[cfg(desktop)]
async fn check_channel(
    app: tauri::AppHandle,
    beta: bool,
) -> tauri_plugin_updater::Result<Option<tauri_plugin_updater::Update>> {
    let endpoint = if beta {
        ${JSON.stringify(endpoints.beta)}
    } else {
        ${JSON.stringify(endpoints.stable)}
    };
    app.updater_builder()
        .endpoints(vec![endpoint.parse()?])?
        .build()?
        .check()
        .await
}`;
}
