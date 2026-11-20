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
