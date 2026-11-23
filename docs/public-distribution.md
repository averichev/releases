# Публичное распространение

Публично доступны только active/public приложения и опубликованные релизы. Любые GET/HEAD/Range/304 проходят PublicDistribution и delivery с проверками владельца, статуса и файла. Полный SHA-256 не вычисляется повторно при скачивании.

| URL | Результат |
| --- | --- |
| `/` | Список active/public приложений с опубликованными релизами; название и логотип ведут на `/{slug}` |
| `/{slug}` | Страница актуальных файлов и истории версий; ссылка на JSON-манифест с кнопкой копирования URL |
| `/{slug}/latest` | 307 на страницу выбора |
| `/{slug}/latest/windows-x86_64` | Максимальный опубликованный SemVer с installer для Windows x64 |
| `/{slug}/latest/windows-aarch64` | То же для Windows ARM64 |
| `/{slug}/latest/darwin-aarch64` | То же для macOS Apple Silicon |
| `/{slug}/latest/darwin-x86_64` | То же для macOS Intel |
| `/{slug}/latest/linux-x86_64` | То же для Linux x64 |
| `/{slug}/latest/windows-x86_64?format=exe` | Максимальная версия с Windows x64 EXE |
| `/{slug}/latest?target=linux-x86_64&type=executable` | Последняя опубликованная версия с Linux x64 бинарником |
| `/{slug}/latest/windows-x86_64?type=installer&format=exe` | Явный выбор установщика Windows EXE |
| `/{slug}/latest/linux-x86_64?type=archive&format=tar.gz` | Последняя версия с подходящим архивом |
| `/{slug}/{version}/{target}?format=exe` | Выбор installer конкретной опубликованной версии |
| `/{slug}/{version}/{target}?purpose=updater` | Явный выбор updater конкретной версии |
| `/{slug}/releases/{version}/artifacts/{uuid}/download` | Постоянная ссылка на конкретный файл |
| `/{slug}/manifest.json?limit=20` | Манифест schema_version=2 |
| `/api/v1/public/applications?limit=20&offset=0` | items (name, slug, icon_url), total, limit, offset; по алфавиту, только active/public с published-релизами |
| `/api/v1/public/applications/{slug}` | name, description, slug, icon_url, manifest_url, latest_by_target, latest_by_type |
| `/api/v1/public/applications/{slug}/releases?limit=20&offset=0` | items, total, limit, offset; опубликованная история по SemVer |
| `/api/v1/public/applications/{slug}/icon` | Иконка приложения |
| `/api/v1/updates/tauri/{slug}/{target}/{arch}/{currentVersion}` | Tauri updater |

Latest выбирает максимальный опубликованный SemVer, содержащий файл с указанными target, типом и format; фильтры применяются до выбора версии. `target` объединяет ОС и архитектуру (`linux-x86_64`, `darwin-aarch64`), отдельные platform/arch не вводятся. `type` — alias существующего `purpose`: `executable|installer|updater|archive`. Одновременная передача type и purpose отклоняется. Без типа сохраняется installer для совместимости старых ссылок; без query `/{slug}/latest` по-прежнему ведёт на страницу выбора. Target/format можно опустить, если выбор однозначен. Prerelease остаются в истории и доступны напрямую, но исключены из latest и автоматического updater. При нескольких подходящих файлах последней подходящей версии — `409 ambiguous_artifact` с предложением указать target, type/purpose и format; произвольного выбора по времени загрузки или отката к более старой однозначной версии нет. Тип никогда не угадывается по расширению. Явный type=updater позволяет скачать пакет; автоматические обновления продолжают использовать отдельный Tauri endpoint с проверкой подписи. Неизвестные query отклоняются. Пустой каталог опубликованных версий возвращает 200 с пустыми структурами.

Публичные artifact DTO: id, filename (каталог), target, format, purpose, size_bytes, sha256, download_url. Release DTO: version, published_at (UTC RFC 3339), notes, artifacts. В latest_by_target ключ — target, значение — Release DTO с установщиками выбранной версии. Дополнительное поле `latest_by_type` в каталоге и manifest v2 содержит `executable|installer|archive → target → Release DTO`: версии для разных типов выбираются независимо. Updater остаётся в истории и Tauri API. Прежнее `latest_by_target` сохраняет только installer. Манифест не включает filename. Limit истории не ограничивает latest_by_target/latest_by_type. CORS публичного JSON/иконок разрешает GET/HEAD/OPTIONS и If-None-Match, без credentials; административные ответы и скачивания CORS не расширяют.

Прямое скачивание поддерживает GET/HEAD, одну Range, 206/416, Content-Length, Content-Disposition, Accept-Ranges, ETag/Last-Modified и условные запросы. Потоковая передача не загружает файл целиком в память, работает с файлами более 2 ГиБ; отмена освобождает ресурсы. Redirect — 307/no-store, файлы не получают долгого public TTL. Скрытое/архивное приложение даёт 404 даже с прежним ETag или Range. Отсутствующий файл, symlink или неверный размер — storage_unavailable; traversal не допускается.

Режим `download.delivery=direct` передаёт файл через Rust. `nginx` использует X-Accel-Redirect только после тех же проверок. Nginx location должен быть internal, alias — к каталогу объектов, disable_symlinks on, mount — read-only. Конфигурация и проверка: [deploy/nginx](../deploy/nginx/README.md), `python3 scripts/smoke_nginx.py` (требуется Docker).

Публичный origin разрешается middleware для текущего запроса: явный download.public_base_url имеет приоритет, иначе проверенный Host и HTTPS от loopback TCP proxy. По тем же правилам сервер формирует абсолютный `manifest_url` для ссылки и копирования на странице приложения. Forwarded/X-Forwarded-Host игнорируются. Домен не сохраняется в общем состоянии. Подробнее: [README](../README.md), [публикация и манифесты](release-management.md), [Tauri](integrations/tauri.md).

## Счётчики скачиваний

Учитывается принятый старт публичной выдачи конкретного артефакта: полный GET или разрешённый Range, начинающийся с нулевого байта. Продолжение с другого байта, HEAD, редиректы latest/version, 304, 412, 416 и другие ошибки не учитываются. Если If-Range не совпал или диапазоны игнорируются и сервер возвращает полный файл, запрос считается новым стартом. Суффиксный Range учитывается, только если охватывает файл с начала. Каталог, manifest/updater JSON и административная проверка/скачивание файлов не меняют статистику; фактическое скачивание пакета обновления по публичному artifact URL учитывается.

Для direct запись происходит после проверок и seek, перед передачей тела; для Nginx — перед X-Accel-Redirect. Разрыв соединения после принятого старта не откатывает счётчик. Ошибка Nginx после делегирования также не наблюдается backend. Повторные старты одного клиента считаются повторно. Метрика не подтверждает полную загрузку, установку или уникального пользователя; Range с нулевого байта при повторной попытке считается снова. Для точного учёта завершений Nginx потребуется отдельный механизм подтверждений.

Счётчик хранится в SQLite с момента миграции 0013; ошибка записи возвращает стандартную ошибку API до выдачи тела. Итоги доступны в панели и существующих административных DTO (`download_count`) по артефакту, версии и приложению; суммы включают все файлы/версии и не зависят от фильтров списка. Публичные JSON, manifest и ETag не зависят от счётчика. Персональных данных и журнала запросов для учёта нет.
