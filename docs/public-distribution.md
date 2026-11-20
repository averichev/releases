# Публичное распространение

Публично доступны только active/public приложения и опубликованные релизы. Любые GET/HEAD/Range/304 проходят PublicDistribution и delivery с проверками владельца, статуса и файла. Полный SHA-256 не вычисляется повторно при скачивании.

| URL | Результат |
| --- | --- |
| `/` | Список active/public приложений с опубликованными релизами; название и логотип ведут на `/{slug}` |
| `/{slug}` | Страница актуальных установщиков каждой платформы и истории |
| `/{slug}/latest` | 307 на страницу выбора |
| `/{slug}/latest/windows-x86_64` | Максимальный опубликованный SemVer с installer для Windows x64 |
| `/{slug}/latest/windows-aarch64` | То же для Windows ARM64 |
| `/{slug}/latest/darwin-aarch64` | То же для macOS Apple Silicon |
| `/{slug}/latest/darwin-x86_64` | То же для macOS Intel |
| `/{slug}/latest/linux-x86_64` | То же для Linux x64 |
| `/{slug}/latest/windows-x86_64?format=exe` | Максимальная версия с Windows x64 EXE |
| `/{slug}/{version}/{target}?format=exe` | Выбор installer конкретной опубликованной версии |
| `/{slug}/{version}/{target}?purpose=updater` | Явный выбор updater конкретной версии |
| `/{slug}/releases/{version}/artifacts/{uuid}/download` | Постоянная ссылка на конкретный файл |
| `/{slug}/manifest.json?limit=20` | Манифест schema_version=2 |
| `/api/v1/public/applications?limit=20&offset=0` | items (name, slug, icon_url), total, limit, offset; по алфавиту, только active/public с published-релизами |
| `/api/v1/public/applications/{slug}` | name, description, slug, icon_url, latest_by_target |
| `/api/v1/public/applications/{slug}/releases?limit=20&offset=0` | items, total, limit, offset; опубликованная история по SemVer |
| `/api/v1/public/applications/{slug}/icon` | Иконка приложения |
| `/api/v1/updates/tauri/{slug}/{target}/{arch}/{currentVersion}` | Tauri updater |

Для каждого target latest считается отдельно; более новая версия другой ОС не скрывает доступный установщик. Prerelease остаются в истории и доступны напрямую, но исключены из latest и автоматического updater. При нескольких форматах без уточнения — `409 ambiguous_artifact`. Роль installer/updater никогда не угадывается по расширению. Неизвестные query отклоняются. Пустой каталог опубликованных версий возвращает 200 с пустыми структурами.

Публичные artifact DTO: id, filename (каталог), target, format, purpose, size_bytes, sha256, download_url. Release DTO: version, published_at (UTC RFC 3339), notes, artifacts. В latest_by_target ключ — target, значение — Release DTO с установщиками выбранной версии. Манифест не включает filename. Limit истории не ограничивает latest_by_target. CORS публичного JSON/иконок разрешает GET/HEAD/OPTIONS и If-None-Match, без credentials; административные ответы и скачивания CORS не расширяют.

Прямое скачивание поддерживает GET/HEAD, одну Range, 206/416, Content-Length, Content-Disposition, Accept-Ranges, ETag/Last-Modified и условные запросы. Потоковая передача не загружает файл целиком в память, работает с файлами более 2 ГиБ; отмена освобождает ресурсы. Redirect — 307/no-store, файлы не получают долгого public TTL. Скрытое/архивное приложение даёт 404 даже с прежним ETag или Range. Отсутствующий файл, symlink или неверный размер — storage_unavailable; traversal не допускается.

Режим `download.delivery=direct` передаёт файл через Rust. `nginx` использует X-Accel-Redirect только после тех же проверок. Nginx location должен быть internal, alias — к каталогу объектов, disable_symlinks on, mount — read-only. Конфигурация и проверка: [deploy/nginx](../deploy/nginx/README.md), `python3 scripts/smoke_nginx.py` (требуется Docker).

Публичный origin разрешается middleware для текущего запроса: явный download.public_base_url имеет приоритет, иначе проверенный Host и HTTPS от loopback TCP proxy. Forwarded/X-Forwarded-Host игнорируются. Домен не сохраняется в общем состоянии. Подробнее: [README](../README.md), [публикация и манифесты](release-management.md), [Tauri](integrations/tauri.md).
