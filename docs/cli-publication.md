# Публикация готового выпуска из curl и CI

Администратор открывает **Приложение → Настройки → Публикация из CLI**, вводит имя (например, `CI MockPort`), при необходимости срок действия и создаёт токен. Секрет с префиксом `vrp_` показывается один раз. Сохраните его в переменной `RELEASES_TOKEN` локально или в secrets CI. Он даёт только публикацию новых выпусков одного приложения. Для обычных неподписанных загрузок токен тоже доступен. UUID продукта, публичный ключ и файлы подписи не заменяют HTTP-аутентификацию.

Одновременно допускаются два действующих токена. Для ротации создайте второй, замените secret в CI, проверьте публикацию и отзовите старый в этом же разделе. Отзыв блокирует в том числе незавершённую загрузку; уже опубликованные релизы и авторство в истории сохраняются. Истёкшие и отозванные токены не занимают активный слот. При потере секрета выпустите новый. Не сохраняйте токен в репозитории, URL, `publish.json`, `release.json` или release notes.

Управление через API (cookie действующей admin-сессии, Origin, X-Veruna-Request и X-CSRF-Token обязательны для mutations):

```text
GET  /api/v1/admin/applications/{application_uuid}/publication-tokens
POST /api/v1/admin/applications/{application_uuid}/publication-tokens
     {"name":"CI MockPort","expires_at":null}
POST /api/v1/admin/applications/{application_uuid}/publication-tokens/{token_id}/revoke
```

POST создания возвращает 201, метаданные и поле `token`; последующие ответы содержат только несекретные метаданные. Даты управления токенами — Unix seconds UTC. Список показывает `active|expired|revoked`. Все ответы имеют `Cache-Control: no-store`. Operator не может ни читать список, ни создавать или отзывать токены. Bearer не даёт доступа к административным API.

## Сетевой контракт

`POST /api/v1/publish/{slug}` принимает `Authorization: Bearer <token>` и multipart/form-data. Сессия браузера не заменяет Bearer. В production нужен HTTPS через доверенный loopback reverse proxy, передающий `Host` и `X-Forwarded-Proto: https`. Внешние подключения не могут объявить себя HTTPS этими заголовками. Явная настройка URL не отключает эту проверку. Для локальной разработки допустим HTTP loopback. CORS-доступ для браузерных mutations не открывается.

Первая часть называется `release` и содержит JSON. Рекомендуемый Content-Type — `application/json`; поддерживается параметр `charset=utf-8` и отсутствие типа у текстового поля с JSON. Содержимое:

```json
{
  "version": "0.3.0",
  "notes": "## Исправления\n- Исправлен повтор загрузки после обрыва.",
  "artifacts": [
    {
      "target": "linux-x86_64",
      "distribution_role": "executable",
      "file_type": "bin",
      "file": "linux_binary",
      "standalone_signature": "linux_jws"
    },
    {
      "target": "darwin-aarch64",
      "distribution_role": "executable",
      "file_type": "bin",
      "file": "mac_binary",
      "standalone_signature": "mac_jws"
    }
  ]
}
```

Остальные части могут идти в любом порядке. `file`, `standalone_signature` и `signature` — имена частей, а не пути. Имена: 1–64 ASCII букв, цифр, `_` или `-`; `release` зарезервировано. Имя скачиваемого файла берётся из multipart filename и проходит общую проверку имени (до 255 UTF-8 байт, без путей и управляющих символов). Используются общие правила SemVer, Markdown notes до 20 000 символов, target, формата и ролей `executable|installer|updater|archive`. Формат нормализуется в нижний регистр и должен соответствовать расширению; поддерживается принятый формат исполняемых файлов без расширения. Справочник target доступен в защищённом `GET /api/v1/admin/artifact-options`.

Для каждого артефакта можно добавить `size_bytes` (положительное целое) и `sha256` (64 строчных hex-символа). Сервер всегда считает фактические значения и отклоняет весь набор при несовпадении. Эти ожидаемые значения не меняют распознавание повтора, если фактические данные одинаковы.

Для standalone передайте готовую `.jws`. Сервер проверяет Ed25519, UUID продукта, полную версию, ОС, архитектуру, фактические размер и SHA-256 по [действующему протоколу](integrations/standalone-protocol.md). UUID и публичный ключ настраиваются в **Подписанные обновления**. Приватный ключ остаётся на машине сборки. Для ручного скачивания уберите `standalone_signature` и соответствующую файловую часть: обычные неподписанные executable/installer/archive разрешены, но не предлагаются standalone updater.

Для роли `updater` обязательны совместимый Tauri-пакет и `.sig` в части, указанной в `signature`. Сервер проверяет структуру подписи и совместимость; криптографическую проверку выполняет Tauri-клиент. Отсутствующая, пустая или нечитаемая `.sig` отклоняет весь выпуск. Подпись Tauri у других ролей и standalone-подпись у updater/archive запрещены.

В именах частей не должно быть повторов; отсутствующие ссылки и незаявленные части запрещены. Единственное совместное использование файловой части: две записи `installer` и `updater` с одинаковыми target и форматом. Обе сохраняются в каталоге с собственными UUID/storage key, байты отправляются и хешируются один раз, `.sig` относится к updater. Другие назначения одной части отклоняются.

## Standalone: команда из каталога готового выпуска

Подготовьте `publish.json` из примера выше и готовые подписанные файлы. Команда выполняется из `releases/0.3.0/`; заголовок передаётся через stdin, поэтому секрет не попадает в argv curl. В CI отключите shell tracing (`set +x`) до работы с секретом.

```sh
printf 'Authorization: Bearer %s\n' "$RELEASES_TOKEN" |
  curl --silent --show-error --fail-with-body --header @- \
    -F 'release=@publish.json;type=application/json' \
    -F 'linux_binary=@mockport-0.3.0-linux-x86_64' \
    -F 'linux_jws=@mockport-0.3.0-linux-x86_64.jws' \
    -F 'mac_binary=@mockport-0.3.0-darwin-aarch64' \
    -F 'mac_jws=@mockport-0.3.0-darwin-aarch64.jws' \
    'https://apps.veruna.systems/api/v1/publish/mockport'
```

Подписи можно подготовить имеющимся [офлайн-примером](integrations/standalone/example.mjs), используя свой ключ и UUID продукта; например из корня репозитория:

```sh
node docs/integrations/standalone/example.mjs sign /secure/private.pem \
  "$PRODUCT_ID" 0.3.0 linux-x86_64 \
  releases/0.3.0/mockport-0.3.0-linux-x86_64 \
  > releases/0.3.0/mockport-0.3.0-linux-x86_64.jws
```

## Tauri: один Windows-пакет для двух ролей

`publish.json`:

```json
{
  "version": "0.3.0",
  "notes": "## Исправления\n- Исправлен повтор загрузки после обрыва.",
  "artifacts": [
    {"target":"windows-x86_64","distribution_role":"installer","file_type":"exe","file":"windows_package"},
    {"target":"windows-x86_64","distribution_role":"updater","file_type":"exe","file":"windows_package","signature":"windows_sig"},
    {"target":"linux-x86_64","distribution_role":"updater","file_type":"appimage","file":"linux_package","signature":"linux_sig"}
  ]
}
```

Из каталога готового выпуска с настоящими `.sig`, созданными Tauri CLI:

```sh
printf 'Authorization: Bearer %s\n' "$RELEASES_TOKEN" |
  curl --silent --show-error --fail-with-body --header @- \
    -F 'release=@publish.json;type=application/json' \
    -F 'windows_package=@MockPort_0.3.0_x64-setup.exe' \
    -F 'windows_sig=@MockPort_0.3.0_x64-setup.exe.sig' \
    -F 'linux_package=@MockPort_0.3.0_amd64.AppImage' \
    -F 'linux_sig=@MockPort_0.3.0_amd64.AppImage.sig' \
    'https://apps.veruna.systems/api/v1/publish/mockport'
```

Windows принимает exe/msi, Linux — appimage, macOS — tar.gz. Настройка публичного ключа Tauri и endpoint клиента описаны в [Tauri integration](integrations/tauri.md).

## release.json и CHANGELOG.md

`release` — сетевое представление, а `release.json` — локальные метаданные сборщика. В запрос не отправляется весь `release.json`: `commit`, локальные пути, конфигурация ключей и другие поля сборки не входят в API. `notes` читается из `CHANGELOG.md` готового выпуска. Если общий changelog содержит много версий, сначала выделите запись нужной версии в файл выпуска.

Формат будущего `veruna-pack` здесь не фиксируется. Ниже полностью рабочий пример адаптации локального состава сборки; если сборщик использует другие имена полей, меняется только адаптер. Существующий `scripts/release.py` этого репозитория записывает в `release.json` только version/commit/previous_tag: состав файлов для него нужно указать отдельно. Доработка сборщика не входит в серверный API.

Пример локального `release.json` с составом:

```json
{
  "version": "0.3.0",
  "artifacts": [
    {"target":"linux-x86_64","distribution_role":"executable","file_type":"bin","filename":"mockport-0.3.0-linux-x86_64","standalone_signature":"mockport-0.3.0-linux-x86_64.jws"},
    {"target":"darwin-aarch64","distribution_role":"executable","file_type":"bin","filename":"mockport-0.3.0-darwin-aarch64","standalone_signature":"mockport-0.3.0-darwin-aarch64.jws"}
  ]
}
```

Этот адаптер запускается из `releases/<версия>/`, строит JSON в памяти и запускает обычный curl. Для просмотра сетевого представления он также сохраняет несекретный `publish.json`. Общий Windows filename автоматически даёт одну файловую часть:

```python
import json, os, subprocess
from pathlib import Path

build = json.loads(Path('release.json').read_text(encoding='utf-8'))
parts = {}
artifacts = []
def part(filename, kind):
    path = Path(filename)
    if path.name != filename or not path.is_file():
        raise ValueError('Ожидался файл в каталоге выпуска')
    # curl -F interprets quotes, commas and semicolons; quote and escape filenames.
    if any(c in filename for c in '\r\n'):
        raise ValueError('Недопустимое имя файла')
    key = (kind, filename)
    if key not in parts:
        parts[key] = f'part_{len(parts)}'
    return parts[key]

for source in build['artifacts']:
    a = {k: source[k] for k in ('target', 'distribution_role', 'file_type')}
    a['file'] = part(source['filename'], 'file')
    for field in ('signature', 'standalone_signature'):
        if source.get(field):
            a[field] = part(source[field], field)
    for field in ('size_bytes', 'sha256'):
        if field in source:
            a[field] = source[field]
    artifacts.append(a)

release = {'version': build['version'], 'notes': Path('CHANGELOG.md').read_text(encoding='utf-8'), 'artifacts': artifacts}
Path('publish.json').write_text(json.dumps(release, ensure_ascii=False), encoding='utf-8')
args = ['curl', '--silent', '--show-error', '--fail-with-body', '--header', '@-',
        '-F', 'release=@publish.json;type=application/json']
for (_, filename), name in parts.items():
    quoted = filename.replace('\\', '\\\\').replace('"', '\\"')
    args += ['-F', f'{name}=@"{quoted}"']
args += ['https://apps.veruna.systems/api/v1/publish/mockport']
subprocess.run(args, input=f"Authorization: Bearer {os.environ['RELEASES_TOKEN']}\n", text=True, check=True)
```

## Результат, повтор и конфликты

Публичная страница и карточка релиза отображают notes как CommonMark Markdown (заголовки, списки, выделение, ссылки, code blocks). Общий компонент на [react-markdown](https://github.com/remarkjs/react-markdown) отключает встроенный HTML и изображения; API и updater получают исходный текст Markdown.

Первая успешная команда возвращает **201**:

```json
{"release_id":"<UUID>","version":"0.3.0","status":"published","published_at":1795521600}
```

`published_at` — Unix seconds UTC. Ответ означает, что опубликованы все артефакты и записано одно событие с автором `Токен CI MockPort`. Версия становится доступной по существующим правилам latest/manifest/standalone/Tauri. Приватное приложение остаётся приватным. Prerelease доступны явно, но исключены из автоматического latest и updater.

При потере ответа повторите ту же команду: **200** с исходными `release_id` и `published_at`. Параллельные одинаковые запросы дают один 201 и остальные 200; в БД остаются один релиз и одно событие. Сравниваются полная нормализованная версия (включая build metadata), нормализованные notes и отсортированные описания файлов: filename, target, формат, роль, фактические размер/хеш и принятые подписи. Имена/порядок частей, порядок артефактов, boundary и конкретный действующий токен не влияют. Сервер заново принимает и проверяет байты перед распознаванием повтора.

Первый опубликованный состав закреплён. Если администратор позже добавил платформу через панель, повтор исходной команды по-прежнему даёт 200. Попытка дополнить выпуск через этот endpoint даёт **409**. Тот же SemVer precedence с другими байтами, notes, именем, ролью, подписями или build metadata тоже даёт 409. Версия, созданная вручную, включая черновик, всегда конфликтует и сохраняется без изменений. Отдельного Idempotency-Key нет: уникальность задаёт `(application_id, version_key)`.

| HTTP | Значение |
|---|---|
| 201 | Весь выпуск впервые опубликован |
| 200 | Точный повтор исходного выпуска |
| 400 | Невалидный multipart/JSON, неверная первая часть |
| 401 `unauthorized` | Токен отсутствует, неверен, истёк или отозван; общая ошибка |
| 403 `forbidden` | Действующий токен использован для чужого/неизвестного slug, либо запрещён транспорт |
| 409 | Конфликт версии, архивное приложение, настройки подписи изменились во время передачи |
| 413 `payload_too_large` | Превышен лимит запроса, файла, JSON или подписи |
| 422 | Невалидные данные, ссылки на части, контрольная сумма или подпись |
| 408 `upload_timeout` | Истёк idle timeout |
| 503 `storage_busy` | Заняты все слоты загрузки; повторить позже |
| 507 `storage_full` | Недостаточно места |

Ошибки используют общий envelope `{"error":{"code":"…","message":"…"}}`; JWS/проверка публикации могут содержать существующие дополнительные детали. Низкий лимит внешнего прокси может дать 413 до backend в формате прокси.

До commit нет видимых черновиков или частичных выпусков: предыдущие latest и манифесты продолжают работать. Ошибка/обрыв освобождает временные и подготовленные объекты; после перезапуска обычный startup reconciliation удаляет бесхозные файлы. Перед commit повторно проверяются токен, приложение и настройки подписи. Длительные передача, SHA-256, проверка подписей и подготовка объектов проходят вне SQLite write-транзакции.

## Лимиты

По умолчанию **весь пакетный запрос** ограничен 2 GiB + 1 MiB (`storage.max_request_bytes`), каждый файл — 2 GiB (`storage.max_file_bytes`). Несколько больших платформ требуют увеличения именно пакетного лимита. Например, четыре файла до 2 GiB:

```toml
[storage]
max_file_bytes = 2147483648
max_request_bytes = 8590983168 # 8 GiB + 1 MiB на весь multipart
upload_idle_seconds = 120
max_concurrent_uploads = 4
```

Те же настройки доступны как `STORAGE__MAX_FILE_BYTES`, `STORAGE__MAX_REQUEST_BYTES`, `STORAGE__UPLOAD_IDLE_SECONDS`, `STORAGE__MAX_CONCURRENT_UPLOADS`. Каждый batch занимает один слот на всю обработку; файловые writer/verify workers используют тот же настроенный предел. Нет ожидания очереди при заполнении — 503. Временное хранилище должно вмещать одновременно принимаемые выпуски.

JSON ограничен 256 KiB; максимум 64 записи артефактов, до 129 частей вместе с `release` (файл и одна подпись на запись); JWS — 4096 байт, Tauri `.sig` — 16384 байт, multipart headers/preamble ограничены отдельным бюджетом 16 KiB. Превышение числа артефактов также даёт 413. Размер подписи считается до обрезки пробельных символов. Idle timeout применяется к чтению заголовков и каждой порции данных, а не ко всему времени передачи.

Для приведённого пакетного лимита настройте действующий HTTPS server/location Nginx:

```nginx
client_max_body_size 8193m;
client_body_timeout 120s;
proxy_request_buffering off;
proxy_read_timeout 300s;
```

Для лимитов по умолчанию достаточно `client_max_body_size 2049m`, как в [конфигурации проекта](../deploy/nginx/veruna.conf). Согласуйте ограничения всех прокси и проверьте `nginx -t` перед reload. URL скачиваний и internal alias не меняются.
