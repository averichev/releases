# Публичное распространение — этап 6

Платформа обслуживает разные продукты одним Rust-бинарником со встроенным React. Примеры VideoLov и Factum ниже не создают данные автоматически: используйте реальные опубликованные релизы и UUID.

## Подготовка

1. В настройках приложения выберите **Публичное**, убедитесь, что оно активно. Private/archived возвращают одинаковый 404 на всех публичных маршрутах, включая условные запросы.
2. В **Распространение** задайте платформу, формат и назначение installer по умолчанию, например `windows-x86_64 / exe / installer`. Настройка относится к приложению, не меняет опубликованные артефакты. При отсутствии выбранного файла проверка публикации показывает предупреждение.
3. Создайте черновик, загрузите реальные файлы. Для updater назначьте роль и загрузите соответствующую `.sig`. Выполните проверку готовности и опубликуйте в stable или beta. При исправлении опубликованного файла создавайте новую версию.
4. Привяжите HTTPS-домен через локальный reverse proxy. Домен панели, манифестов и updater определяется автоматически из проверенного Host; HTTPS подтверждается X-Forwarded-Proto только от loopback TCP peer. «Мой хост» передаёт эти заголовки автоматически. Явные `download.public_base_url` и `auth.public_origin` нужны только для фиксированных адресов особого развёртывания.

```toml
[download]
# public_base_url = "https://downloads.example.com" # необязательный отдельный домен
delivery_mode = "direct" # или nginx с конфигурацией deploy/nginx
max_concurrent_downloads = 64
```

По умолчанию URL определяется из текущего запроса; delivery — direct, 64 одновременных потока. URL должен быть HTTPS origin без пути/trailing slash/credentials/query; HTTP разрешён только на loopback. Переменные: `DOWNLOAD__PUBLIC_BASE_URL`, `DOWNLOAD__DELIVERY_MODE`, `DOWNLOAD__MAX_CONCURRENT_DOWNLOADS`. При загрузке файлов больше 2 GiB увеличьте также `storage.max_file_bytes`, `storage.max_request_bytes` и лимит reverse proxy.

## Маршруты

Все перечисленные маршруты read-only, поддерживают GET/HEAD; HEAD не передаёт тело. Публичная страница `/{slug}` использует то же React-приложение без session provider и административной навигации. Прямое открытие неизвестного/private/archived slug даёт 404 HTML с экраном ошибки. Ошибки API и файлов — JSON, без SPA fallback. Для базовой страницы нужен JS; полученные ссылки скачивания работают обычными HTML anchors без JS.

| URL | Поведение |
| --- | --- |
| `/{slug}` | Страница stable: описание, иконка, выбор ОС/архитектуры/формата, notes, история |
| `/{slug}?channel=beta` | Явно выбранная beta в интерфейсе |
| `/{slug}/latest` | Разрешение stable → 307 на постоянную ссылку |
| `/{slug}/latest?channel=beta&target=linux-x86_64&format=appimage` | Явный канал и селектор → 307 |
| `/{slug}/{version}/{target}?format=exe` | Опубликованная версия в stable → 307 |
| `/{slug}/{version}/{target}?channel=beta&purpose=updater` | Явно выбранный beta/updater |
| `/{slug}/releases/{version}/artifacts/{artifactId}/download` | Конкретный файл, независимо от текущего указателя канала |
| Та же ссылка с `?channel=beta` | Проверяет неизменяемое членство версии в beta; нужна для beta-only релизов |
| `/{slug}/manifest.json?channel=stable&limit=20` | Универсальный манифест схемы 1 с рабочими download_url |
| `/api/v1/public/applications/{slug}?channel=stable` | Название, описание, slug, icon_url, channel, latest или null, platforms, default_artifact_id |
| `/api/v1/public/applications/{slug}/releases?channel=stable&limit=20&offset=0` | `{items,total,limit,offset}`; limit 1–100, offset 0–100000 |
| `/api/v1/public/applications/{slug}/icon` | Публичная PNG-иконка через IconStorage |
| `/api/v1/updates/tauri/{slug}/{channel}/{target}/{arch}/{currentVersion}` | Динамический Tauri v2: 200 JSON или 204 без тела |

Канал — stable по умолчанию, beta только явно. История и разрешение конкретной версии проверяют членство в выбранном канале. Переключение/rollback меняют только current: старые членства, артефакты и ссылки сохраняются. Ссылки из beta-манифеста сохраняют `?channel=beta` даже после продвижения релиза в stable. Конкретная версия в URL — полная сохранённая SemVer, включая build metadata. UUID без правильных slug/версии не даёт доступ к файлу другого приложения.

`latest` содержит `{version,published_at,notes,artifacts}`; artifact содержит `{id,filename,target,format,purpose,size_bytes,sha256,download_url}`. `published_at` — RFC3339. Нет storage keys, путей, подписей, авторов или административных timestamps/revision. Приложение без публикаций доступно с latest=null и пустой историей. Beta-only файлы не появляются в stable API.

## CORS

Манифест `/{slug}/manifest.json`, публичное API `/api/v1/public/applications/{slug}` (включая `/releases` и `/icon`) и Tauri updater разрешают чтение с любого домена: `Access-Control-Allow-Origin: *`, методы GET/HEAD, без credentials. OPTIONS обслуживает preflight; разрешён заголовок `If-None-Match` для условного запроса манифеста. `ETag` доступен JavaScript через `Access-Control-Expose-Headers`. CORS сохраняется для 204, 304, 404 и остальных ошибок этих маршрутов; правила кеширования и проверки публичности действуют как обычно.

Пример для страницы на `https://videolov.ru` или `https://factum.systems` (замените slug на нужное приложение):

```js
const response = await fetch(
  'https://apps.veruna.systems/videolov/manifest.json',
  { credentials: 'omit' },
);
if (!response.ok) throw new Error(`Manifest HTTP ${response.status}`);
const etag = response.headers.get('ETag');
const manifest = await response.json();
console.log(manifest, etag);
```

При повторном fetch можно передать `headers: { 'If-None-Match': etag }`, если ETag получен; ответ 304 не содержит JSON, используйте ранее сохранённый манифест. Административное API и аутентификация CORS не разрешают; Origin/CSRF и cookie не меняются. Для обычной загрузки бинарного файла по ссылке CORS не требуется.

## Выбор файла

Фильтры `target`, `format`, `purpose` применяются первыми, purpose по умолчанию installer. После фильтрации один файл выбирается автоматически. При нескольких кандидатах применяется сохранённый селектор, если он однозначно совпал среди кандидатов. Если совпадения нет, ответ `409 ambiguous_artifact`; настройка не подменяет явно выбранную пользователем платформу. Без подходящего файла — 404. Неверный селектор — 422. User-Agent не участвует.

Роль updater не означает обычный установщик. `/latest` не выдаёт updater без `purpose=updater`; основной блок страницы показывает installer, история подписывает назначение всех файлов. По существующей модели у файла одна роль и уникальная пара target/format. Если Windows EXE используется Tauri, назначьте updater и используйте его постоянную ссылку для ручной загрузки либо другой installer-формат (например MSI) для `/latest`. Новая двойная роль в этом этапе не введена.

## Передача и кеширование

`PublicDistribution.current` разрешает явный указатель канала, `select_artifact` выбирает файл, `PublicDownload` содержит приложение/релиз/артефакт/канал и проходит единую проверку доступа перед delivery. Эти операции используют те же Release/Artifact/ReleaseChannel, что публикации и манифесты.

Direct открывает только доверенный storage key через ArtifactStorage: capability directory, no-follow symlinks, regular-file и зарегистрированный размер. Чтение через Tokio/ReaderStream, блок 64 КиБ, Content-Length в u64, ограничение конкурентных потоков. Body владеет файлом/permit; disconnect освобождает их. HEAD проверяет файл и возвращает метаданные без чтения тела. MIME определяется по имени, неизвестный — application/octet-stream; всегда attachment + RFC 5987 UTF-8 filename*, безопасный ASCII fallback и nosniff. Компрессия скачиваний отключена для сохранения байтов/Range.

Размер проверяется при каждом запросе; SHA-256 — при загрузке и публикации, по отдельной административной команде. Повторного хеширования на каждом GET/HEAD/manifest/updater нет. Внешнее изменение файла того же размера нельзя обнаружить дешёвым HEAD: опубликованное хранилище считается неизменяемым и не допускает посторонней записи. Восстановление backup должно сохранять содержимое и mtime.

`/latest` и version redirect — `307` + `no-store`. Файлы — `private, no-cache, must-revalidate`, сильный ETag в формате Nginx `"mtime_hex-size_hex"`, Last-Modified. Длительных публичных TTL нет: следующий запрос снова проверяет публичность, включая перед 304. Уже полученную копию и уже открытый поток невозможно отозвать. Манифест — слабый ETag от сериализованного JSON, включая URL; смена канала/представления изменяет ETag, схема остаётся 1. API — no-store.

Поддержаны If-Match/If-Unmodified-Since (412), If-None-Match/If-Modified-Since (304), одиночный Range и If-Range (сильный ETag либо точная дата Last-Modified). Несовпавший/слабый If-Range игнорирует Range и даёт полный 200. Range применяется только к GET. Открытый, закрытый, suffix, последний байт и выход end за конец поддерживаются. Невозможный/некорректный одиночный byte range — 416 с `Content-Range: bytes */SIZE`; неизвестные единицы и multiple ranges игнорируются с 200 согласно разрешённому поведению [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html#name-range). Не формируется multipart.

Nginx: см. [конфигурацию и права](../deploy/nginx/README.md). Rust проверяет доступ и файл, выдаёт доверенный `X-Accel-Redirect`, Nginx читает файл через internal alias. Публичные URL, Content-Disposition, кеширование и валидаторы сохраняются. Аналитики нет: PublicDownload — место будущего контекста запроса, method/status остаются на HTTP-границе. Redirect/HEAD/Range/retry не считаются отдельными установками. Nginx access log сохраняет время/метод/статус/байты без URL и query.

## Диагностика и примеры

В **Распространение → Проверить** выберите stable/beta. Браузер читает манифест и делает HEAD артефактов только по проверенным маршрутам текущего сервиса, без загрузки тела дистрибутива и без произвольных исходящих адресов. Проверяются версия канала, структура URL, статус/размер файлов; Tauri preview диагностирует подпись и выбор платформы. Для других платформ используйте **Манифесты → Tauri preview**. Ошибки backend не раскрывают пути.

```sh
curl -I 'https://apps.veruna.systems/videolov/latest?target=windows-x86_64&format=msi'
curl -fL 'https://apps.veruna.systems/factum/latest?target=linux-x86_64&format=deb' -o factum.deb
curl -f 'https://apps.veruna.systems/videolov/manifest.json?channel=beta'
curl -f 'https://apps.veruna.systems/factum/1.4.0/linux-x86_64?format=deb' -L -o factum-1.4.0.deb
# Замените UUID реальным значением из API:
URL='https://apps.veruna.systems/videolov/releases/1.4.0/artifacts/РЕАЛЬНЫЙ_UUID/download'
curl -I "$URL"
curl -D - -H 'Range: bytes=0-1023' "$URL" -o part.bin
curl -C - -o setup.exe "$URL"
curl -i 'https://apps.veruna.systems/videolov/latest?channel=beta&target=windows-x86_64&purpose=updater'
```

Если 404 — проверьте активность/публичность, публикацию, версию/UUID и членство stable/beta. 409 — уточните выбор. 503 storage_unavailable — файл отсутствует/размер неверен; восстановите согласованный backup. 503 updater_unavailable — пакет неоднозначен или подпись отсутствует/невалидна; откройте preview. 204 updater — канал пуст, версия клиента не ниже current либо для корректного target нет updater-пакета. Rollback канала не понижает клиентскую версию.

Не реализованы статистика, CI/CD-токены, private links/updates, лицензии, downgrade, delta и внешние хранилища.

Проверки этапа: `sh scripts/check.sh`, `sh scripts/build.sh`, `python3 scripts/smoke.py`; дополнительная реальная проверка Nginx — `python3 scripts/smoke_nginx.py`. Тесты используют временные SQLite/хранилища, не устанавливают фиктивные релизы в рабочую БД. Большой размер проверяется sparse-файлом 3 GiB, без загрузки гигабайтов из сети. Полная установка Tauri проверяется отдельно на реальном клиентском проекте с вашим ключом, см. инструкцию интеграции.
