import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

let updating = false;

// report выводит статус в интерфейс приложения, например в текст под кнопкой.
export async function checkForUpdates(
  report: (message: string) => void,
  confirmInstall: (version: string) => Promise<boolean> = async (version) =>
    window.confirm(`Доступна версия ${version}. Установить и перезапустить приложение?`),
): Promise<void> {
  if (updating) return;
  updating = true;
  let update: Awaited<ReturnType<typeof check>> = null;
  let installed = false;
  try {
    report('Проверяем обновления…');
    update = await check({ timeout: 30_000 });
    if (!update) {
      report('Для этой платформы обновлений нет.');
      return;
    }
    report(`Доступна версия ${update.version} (установлена ${update.currentVersion}).`);
    if (!(await confirmInstall(update.version))) return;
    let downloaded = 0;
    let total: number | undefined;
    await update.downloadAndInstall((event) => {
      if (event.event === 'Started') {
        total = event.data.contentLength;
        report('Начинаем загрузку…');
      } else if (event.event === 'Progress') {
        downloaded += event.data.chunkLength;
        report(total
          ? `Загружено ${Math.min(100, Math.round(downloaded / total * 100))}%`
          : `Загружено ${downloaded} байт`);
      } else {
        report('Загрузка завершена. Проверяем подпись и устанавливаем…');
      }
    });
    installed = true;
    // На Windows установщик завершает процесс; этот код может не выполниться.
    // На Linux и macOS нужен перезапуск для запуска новой версии.
    report('Обновление установлено. Перезапускаем приложение…');
    await relaunch();
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    report(installed
      ? `Обновление установлено, но перезапуск не выполнен. Перезапустите приложение вручную: ${detail}`
      : `Ошибка обновления: ${detail}`);
  } finally {
    if (update && !installed) {
      try { await update.close(); }
      catch { report('Не удалось освободить ресурс обновления. Перезапустите приложение перед повторной попыткой.'); }
    }
    updating = false;
  }
}
