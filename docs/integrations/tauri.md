# Tauri Updater v2 и Veruna Releases

Протокол соответствует [официальному Tauri v2 Updater](https://v2.tauri.app/plugin/updater/): 200 JSON с version/url/signature или 204 без тела. Public endpoint не использует административные cookie. Приложение должно быть активным/public, релиз опубликован в указанном канале. HTTPS обязателен для production клиента.

## Клиент VideoLov

В проекте VideoLov установите plugin updater (`npm run tauri add updater`) и, если нужен перезапуск из JS, plugin process. Регистрируйте плагины в Rust согласно документации Tauri. В capability окна разрешите `updater:default` и `process:allow-restart` для используемых команд. Не выдавайте updater-команды недоверенному удалённому контенту.

Фрагмент реальной конфигурации `src-tauri/tauri.conf.json` для интеграции VideoLov (объединить с существующими app/bundle/identifier; значение pubkey заменить содержимым **своего публичного** ключа, без пути к файлу):

```json
{
  "productName": "VideoLov",
  "version": "1.3.0",
  "bundle": {
    "active": true,
    "createUpdaterArtifacts": true
  },
  "plugins": {
    "updater": {
      "pubkey": "ВСТАВЬТЕ_СОДЕРЖИМОЕ_СВОЕГО_ПУБЛИЧНОГО_КЛЮЧА",
      "endpoints": [
        "https://apps.veruna.systems/api/v1/updates/tauri/videolov/stable/{{target}}/{{arch}}/{{current_version}}"
      ],
      "windows": { "installMode": "passive" }
    }
  }
}
```

Это шаблон для настоящего проекта; ключ и версия должны соответствовать вашей сборке. Приватный ключ или пример «рабочей подписи» здесь не приводятся. Для Factum используйте тот же контракт с `/factum/stable/…`, если Factum действительно использует Tauri; обычному CLI/server продукту этот plugin не нужен.

Tauri подставляет `{{target}}` = windows/linux/darwin, `{{arch}}` = x86_64/aarch64/i686/armv7, `{{current_version}}` = SemVer установленного клиента. Каталог загрузки Veruna сейчас принимает windows/linux/darwin x86_64/aarch64 и any; i686/armv7 распознаются протоколом, но пакеты этих платформ пока не загружаются через каталог. Для корректной, но отсутствующей платформы endpoint даёт 204, для некорректных target/arch — 422. Канал фиксируется в endpoint: beta-клиенту задайте `/beta/`. `{{channel}}` не является подстановкой Tauri.

## Сборка и подписи

1. Сгенерируйте пару ключей на машине сборки: `npm run tauri signer generate -- -w ~/.tauri/videolov.key`. Сохраните приватный ключ в защищённом месте; его потеря блокирует последующие обновления старых клиентов. Публичный ключ встроен в приложение, Veruna не хранит ключи подписи.
2. Для сборки задайте `TAURI_SIGNING_PRIVATE_KEY` (путь либо содержимое приватного ключа) и при необходимости `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` в окружении процесса сборки. `.env` Tauri для этого не использует. Выполните `npm run tauri build` с createUpdaterArtifacts=true.
3. В черновике Veruna загрузите файл пакета и выберите правильный target/format. Явно назначьте роль **Tauri updater**. Загрузите соответствующий файл `.sig`: API сохраняет его настоящее base64-содержимое и проверяет структуру minisign. SHA-256 не является подписью. Криптографическая проверка пары пакет/подпись выполняется клиентом с встроенным публичным ключом; сервер не может подтвердить авторство без ключа и не притворяется, что проверил его.
4. Проверьте готовность и опубликуйте в beta. В **Манифесты → Tauri preview** задайте установленную версию/target. Ответ available содержит реальную постоянную ссылку и сохранённую подпись; missing signature/ambiguous/corrupted объясняют блокировку.
5. Проверьте HTTP и тестовый клиент. Затем назначьте тот же опубликованный релиз stable, не перезагружая файл. Подпись/роль опубликованного файла изменить нельзя: ошибка требует нового релиза.

| Платформа | Файл для createUpdaterArtifacts=true | Target/format в Veruna |
| --- | --- | --- |
| Windows | Подписанный NSIS setup.exe или MSI и соответствующий .sig | windows-x86_64/aarch64; exe/msi |
| Linux | Сам AppImage и AppImage.sig | linux-x86_64/aarch64; appimage |
| macOS | app.tar.gz из bundle/macos и app.tar.gz.sig | darwin-x86_64/aarch64; tar.gz |

На macOS DMG не подходит updater, используйте отдельный installer DMG для ручного скачивания. Windows EXE должен быть NSIS-сборкой Tauri, одно расширение не доказывает совместимость. На Linux DEB/RPM не являются Tauri updater. v1Compatible ZIP/tar.gz для миграции старого Tauri не поддержаны этим адаптером. Notarization/code signing ОС — отдельная часть сборки, не заменяется подписью updater. Файл с ролью updater не выбирается обычным `/latest` без purpose=updater; его постоянная ссылка пригодна и для ручной загрузки.

## Проверка

```sh
curl -i 'https://apps.veruna.systems/api/v1/updates/tauri/videolov/beta/windows/x86_64/1.3.0'
# Если beta новее, ответ 200, version/pub_date/url/signature/notes.
# Полученный url: HEAD и затем загрузка клиентом с криптографической проверкой.
curl -i 'https://apps.veruna.systems/api/v1/updates/tauri/videolov/stable/windows/x86_64/99.0.0'
# 204 без тела, даже после rollback канала.
curl -i 'https://apps.veruna.systems/api/v1/updates/tauri/videolov/stable/windows/x86_64/not-semver'
# 422, не ложное «нет обновления».
```

Минимальный код существующего клиента после регистрации plugins/capabilities:

```ts
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

const update = await check();
if (update) {
  await update.downloadAndInstall();
  await relaunch();
}
```

В реальном интерфейсе запрашивайте согласие пользователя и показывайте прогресс/ошибки согласно UX приложения. Для проверки установите **собранный и подписанный** клиент 1.3.0, опубликуйте совместимый 1.4.0 с тем же ключом, запустите check/download/install, после рестарта проверьте app version. Повторный check должен вернуть отсутствие обновления. Не переопределяйте client version comparator для downgrade.

Сервер сравнивает SemVer по precedence (build metadata не делает новую версию), затем проверяет ровно один совместимый updater и наличие сохранённой подписи. 204 означает пустой канал, отсутствие более новой версии или отсутствие updater для валидной платформы. Несуществующее/private/archived приложение — 404; невалидный SemVer/target — 422; ошибочная конфигурация подписи/неоднозначность — 503 updater_unavailable. Отсутствие/размер файла — 503 storage_unavailable. Публичный ответ никогда не содержит 200 с придуманным URL или неподписанным пакетом.

Автотесты Veruna проверяют HTTP/JSON/подстановки, exact target, подпись, URL, stable/beta, downgrade и 204 без тела. Они используют синтаксические тестовые подписи и не имитируют криптографический успех установки. Полная end-to-end установка требует реального Tauri проекта, ключа и совместимой ОС; этот репозиторий не содержит отдельного демонстрационного приложения.
