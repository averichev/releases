# Nginx delivery

Включите `veruna.conf` внутри `http {}`. Измените пути сертификатов и `alias`: последний должен указывать **точно** на `storage.directory`, без `.tmp`, icons или БД. Rust слушает loopback, наружу открыт только Nginx. Nginx перезаписывает Host и X-Forwarded-Proto; при локальном upstream домен и HTTPS определяются автоматически. В TOML:

```toml
[download]
delivery_mode = "nginx"
max_concurrent_downloads = 64
```

В примере задан `client_max_body_size 2049m`: 2 GiB на файл и 1 MiB на multipart. Примените его к действующему HTTPS `server` нужного домена; более низкий лимит в `location` или внешнем прокси также может отклонить загрузку. Размер в форме показывает ограничение backend. Ответ HTTP 413 с HTML вместо JSON обычно означает отказ прокси до backend: панель показывает статус и предлагает проверить лимит прокси. Проверьте конфигурацию через `nginx -t` перед reload.

Пользователь Rust владеет каталогом, создаёт и удаляет файлы. Рабочий пользователь Nginx получает только чтение файлов и проход родительских каталогов (например, отдельная read-only группа: каталог 0750, файлы 0640 с соответствующим umask). Nginx не получает доступ к БД, `.tmp`, icons и секретам. Запись в хранилище разрешена только доверенному процессу Rust/владельцу сервера; каталоги не могут заменяться непривилегированными пользователями. `disable_symlinks on`, проверка capability-scoped open в Rust и строгий формат ключа защищают обе стадии передачи. Между проверкой Rust и открытием Nginx нет общей транзакции: администратор ОС не должен менять/заменять опубликованные файлы. Повреждённый после проверки файл требует восстановления из согласованной резервной копии.

`internal` обязателен. Не добавляйте другой публичный alias, `try_files` или static root для хранилища. X-Accel-Redirect обрабатывается Nginx, физический путь клиенту не передаётся. HEAD, 304, 412 и 416 Rust возвращает без делегирования; полный GET делегируется, диапазон обрабатывает стандартный range filter Nginx. `max_ranges 1` соответствует direct: multiple ranges игнорируются с полным 200. Резервный direct включается одной настройкой, URL не меняются. ETag одинаков при сохранении mtime файла.

Проверка после `nginx -t` и reload, на опубликованном тестовом файле:

```sh
URL='https://apps.veruna.systems/videolov/releases/1.4.0/artifacts/РЕАЛЬНЫЙ_UUID/download'
curl -I "$URL"                          # 200, длина, ETag, attachment
curl -D /tmp/headers -H 'Range: bytes=0-3' "$URL" -o /tmp/part
wc -c /tmp/part                         # 4; 206 + bytes 0-3/SIZE
curl -i -H 'Range: bytes=999999999999-' "$URL"   # 416 + bytes */SIZE
# Подставьте ETag из HEAD; совпавший If-Range даёт 206, другой — полный 200.
curl -D - -H 'Range: bytes=-4' -H 'If-Range: "ПОЛУЧЕННЫЙ_ETAG"' "$URL" -o /tmp/part
curl -i 'https://apps.veruna.systems/_veruna_files/00000000000000000000000000000000.blob' # 404
```

Затем сделайте приложение private/archived: тот же URL (включая If-None-Match) должен дать 404. Нельзя получать файл по UUID другого приложения или черновика. Логи Nginx фиксируют байты передачи и статус, не считают установки/скачивания; Range/retry может дать несколько записей. Rust ограничивает только подготовку делегирования, активные передачи Nginx ограничиваются `limit_conn`.

Автоматический Docker smoke с временной SQLite и тестовым хранилищем: `python3 scripts/smoke_nginx.py`. Требует доступного Docker и образа `nginx:stable-alpine`; не касается production. На macOS использует `host.docker.internal`, на Linux — host-gateway. Обычный `scripts/smoke.py` не требует Docker.
