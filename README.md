# Veruna Releases

Самостоятельная платформа распространения ПО. Этап 1: HTTP-сервер, встроенная панель администратора, SQLite, конфигурация и инфраструктура следующих этапов.

Каталог приложений, релизы, загрузка файлов, манифесты, статистика и авторизация **не реализованы**. Панель содержит заглушки без фиктивных данных. `/api/v1/admin` и все вложенные пути закрыты с `503 administration_unavailable` до реализации авторизации.

## Требования

- Rust 1.91+ с Cargo, rustfmt и Clippy; системный C toolchain для сборки SQLite.
- Node.js 20.19+ или 22.12+ и npm; для нового окружения используйте поддерживаемую LTS-ветку Node.js, совместимую с `frontend/package.json`.
- Python 3 — только для smoke-теста, без сторонних библиотек.

Версии зависимостей фиксируются `Cargo.lock` и `frontend/package-lock.json`. SQLx 0.8.6 выбран для совместимости с Rust 1.91; переход на 0.9 требует отдельного обновления Rust. SQLite собирается вместе с SQLx: системный сервер БД и SQLx CLI не нужны.

## Быстрый запуск

Все команды ниже выполняются из корня проекта:

```sh
npm --prefix frontend ci
npm --prefix frontend run build
cargo run --locked
```

Открыть [панель](http://127.0.0.1:8080/admin). Проверить сервер:

```sh
curl --fail http://127.0.0.1:8080/api/v1/health
```

Ответ `200 application/json` после успешного `SELECT 1`:

```json
{"status":"ok","service":"veruna-releases","version":"0.1.0"}
```

Если база недоступна — `503` и `{"error":{"code":"database_unavailable","message":"Service is not ready"}}`. Это проверка готовности HTTP/БД, а не полный аудит диска или внешних зависимостей.

При первом запуске автоматически создаются `data/veruna.sqlite`, родительские каталоги и `data/artifacts`. Миграции применяются до открытия HTTP-порта. Ошибки конфигурации, миграций, файловой системы или привязки порта завершают процесс с ненулевым кодом.

## Разработка

Сначала один раз выполните сборку фронтенда из быстрого запуска. Затем два терминала:

```sh
# Терминал 1: backend; после изменений Rust перезапустите команду.
cargo run --locked
```

```sh
# Терминал 2: Vite с HMR.
npm --prefix frontend run dev
```

Для разработки интерфейса открывайте [Vite](http://127.0.0.1:5173). Он проксирует `/api` на `http://127.0.0.1:8080`, поэтому CORS не требуется. При другом адресе backend:

```sh
VERUNA_DEV_API_TARGET=http://127.0.0.1:9090 npm --prefix frontend run dev
```

Порт Rust продолжает обслуживать **последнюю встроенную сборку**. Чтобы обновить её, выполните `npm --prefix frontend run build` и перезапустите `cargo run --locked`. Ресурсы встроены и в debug-, и в release-бинарник. `build.rs` отслеживает `frontend/dist` и миграции; без предварительно собранного фронтенда Cargo завершится с инструкцией по сборке. Cargo не запускает npm и не устанавливает JS-зависимости неявно.

## Конфигурация

Приоритет: встроенные значения → `veruna.toml` → переменные `VERUNA__…`. Конфигурационный файл необязателен; скопируйте `veruna.example.toml` в `veruna.toml`, если нужна локальная настройка. `.env` автоматически не загружается.

Можно выбрать другой TOML-файл:

```sh
VERUNA_CONFIG=/etc/veruna/releases.toml ./target/release/veruna-releases
```

Явно указанный файл обязан существовать. Неизвестные поля и опечатки в `VERUNA__…`, неверные типы и недопустимые значения приводят к ошибке запуска. Относительные пути всегда отсчитываются от **рабочего каталога процесса**, а не от расположения TOML или бинарника. Для production используйте абсолютные пути.

| TOML | Переменная | По умолчанию |
| --- | --- | --- |
| `server.bind` | `VERUNA__SERVER__BIND` | `127.0.0.1:8080` |
| `server.shutdown_timeout_seconds` | `VERUNA__SERVER__SHUTDOWN_TIMEOUT_SECONDS` | `15` |
| `database.path` | `VERUNA__DATABASE__PATH` | `data/veruna.sqlite` |
| `database.max_connections` | `VERUNA__DATABASE__MAX_CONNECTIONS` | `4` |
| `database.busy_timeout_seconds` | `VERUNA__DATABASE__BUSY_TIMEOUT_SECONDS` | `5` |
| `storage.directory` | `VERUNA__STORAGE__DIRECTORY` | `data/artifacts` |
| `logging.filter` | `VERUNA__LOGGING__FILTER` | `info,tower_http=info` |

`server.bind` — IP и порт, например `127.0.0.1:8080`, `0.0.0.0:8080` или `[::1]:8080`. Размер пула — 1–32, таймауты — 1–300 секунд. Логи JSON выводятся в stdout; HTTP-логи содержат метод, путь, статус и длительность, без query string и тел запросов. Фильтр настраивается через TOML или `VERUNA__LOGGING__FILTER`; `RUST_LOG` не используется.

## Production

```sh
sh scripts/build.sh
python3 scripts/smoke.py
```

Результат — `target/release/veruna-releases`: скрипт явно задаёт `--target-dir target`, в том числе при пользовательских глобальных настройках Cargo. Перенесите на совместимую ОС и архитектуру **только бинарник**, при необходимости добавив TOML. Для запуска не нужны Node.js, `frontend/dist`, исходники или каталог миграций. Внешние данные — SQLite и каталог дистрибутивов — остаются на диске.

```sh
VERUNA__SERVER__BIND=127.0.0.1:8080 \
VERUNA__DATABASE__PATH=/var/lib/veruna-releases/database/releases.sqlite \
VERUNA__STORAGE__DIRECTORY=/var/lib/veruna-releases/artifacts \
./veruna-releases
```

Процесс должен иметь права создавать каталоги и файлы в указанных местах. SQLite работает в `WAL` с `foreign_keys=ON` для каждого соединения, `synchronous=FULL` и ожиданием блокировки. Используйте локальный диск; рядом с БД могут появляться `-wal` и `-shm`. Каталог дистрибутивов не публикуется как статический каталог. Не размещайте данные в `frontend/public` или `frontend/dist`.

Предполагаемая установка — `https://apps.veruna.systems` в корне домена. TLS завершается на reverse proxy, который передаёт **все** пути Rust-серверу. Например, Caddy:

```caddyfile
apps.veruna.systems {
    reverse_proxy 127.0.0.1:8080
}
```

Отдельный веб-сервер для assets не требуется. Префикс вроде `/releases/` сейчас не поддерживается. `/admin` на этом этапе — публично видимый каркас интерфейса без приватных данных; закрытая граница операций находится в `/api/v1/admin`. Это не реализация аутентификации.

`Ctrl+C`/SIGINT и SIGTERM на Unix останавливают приём соединений, дают текущим запросам до `shutdown_timeout_seconds` на завершение, затем закрывают пул БД с таким же отдельным пределом. Превышение лимита завершается ошибкой. Таймаут остановки service manager должен превышать сумму двух лимитов; при значении 15 секунд задайте не менее 35 секунд.

## Проверки

```sh
sh scripts/check.sh
sh scripts/build.sh
python3 scripts/smoke.py
```

`check.sh` устанавливает зависимости по lock-файлу, проверяет TypeScript, собирает UI, запускает Vitest, `cargo fmt --check`, Rust-тесты и `cargo clippy --all-targets -- -D warnings`.

Тесты проверяют приоритет и ошибки конфигурации, реальные файловые пути, настройки соединений SQLite, повторное применение миграций, отсутствие бизнес-таблиц, health и его отказ, блокировку административного API, JSON-ошибки, SPA-маршруты, статические ресурсы и HEAD. Фронтенд проверяет контракт health, состояние ошибки и навигацию. Smoke-тест запускает production-бинарник из временного пустого каталога, загружает HTML/JS/CSS, проверяет БД и завершает процесс SIGTERM.

Архитектура и развитие: [docs/architecture.md](docs/architecture.md). Правила внесения изменений: [AGENTS.md](AGENTS.md).
