# Административное API каталога

Базовый путь — `/api/v1/admin/applications`. Все запросы требуют действующую session cookie. Чтение доступно admin/operator; изменения — только admin, с точным `Origin`, `X-Veruna-Request: 1` и `X-CSRF-Token` из login/me. Авторизация и управление пользователями описаны в [README](../README.md#http-api-этапа-2).

| Метод | Путь относительно базового | Результат |
| --- | --- | --- |
| GET | пустой | 200, `{items,total,page,per_page}` |
| POST | пустой | 201, Application DTO |
| GET | `/{id}` | 200, Application DTO |
| PATCH | `/{id}` | 200, Application DTO |
| POST | `/{id}/archive` | 200, архивный Application DTO |
| POST | `/{id}/restore` | 200, активный Application DTO |
| GET / HEAD | `/{id}/icon` | 200, нормализованный PNG; 404 без иконки |
| POST | `/{id}/icon` | 200, Application DTO с новым icon_url |
| DELETE | `/{id}/icon` | 200, Application DTO с icon_url = null |

`id` — постоянный UUID. Физическое удаление приложений отсутствует; DELETE `/{id}` возвращает 405.

## Список

| Query | Допустимые значения | По умолчанию |
| --- | --- | --- |
| `page` | целое от 1 | 1 |
| `per_page` | целое 1–100 | 24 |
| `search` | до 120 Unicode-символов, буквальная подстрока названия/slug без учёта регистра, включая кириллицу | пусто |
| `status` | active, archived, all | active |
| `visibility` | private, public, all | all |
| `sort` | name, created_at, updated_at | updated_at |
| `order` | asc, desc | desc |

Пример: `GET /api/v1/admin/applications?status=all&visibility=public&search=crm&sort=name&order=asc&page=1&per_page=24`. Одинаковые ключи сортировки разрешаются через UUID; count и items берутся из одного snapshot. Страница вне диапазона содержит пустой items с реальным total. Неизвестные параметры и неверные значения — 400.

## Создание и изменение

POST принимает JSON:

```json
{"name":"Просто CRM","slug":"prosto-crm","description":"Система для команды","visibility":"private"}
```

Обязательны name и slug. Description по умолчанию пустое, visibility — private, status всегда active. Name после trim содержит 1–120 Unicode-символов без управляющих символов. Description после trim — до 2000 символов; переводы строк и табуляция допустимы. Slug — 1–64 символа a-z, 0-9 и дефис внутри; регистр не нормализуется молча. Зарезервированы api, admin, assets, auth, login, logout, static, health, activate, favicon, index. Уникальность распространяется и на архив.

PATCH принимает непустое подмножество name, description, visibility. `id`, `slug`, `status`, `icon_key`, даты, неизвестные поля и явный null запрещены. Даже передача прежнего slug — 400. Статус меняется только через archive/restore. Публичность при этом сохраняется. Повторные archive/restore и удаление уже отсутствующей иконки безопасны. Лимит JSON каталога — 32 KiB; остальные identity endpoints сохраняют предел 8 KiB.

## DTO

Application DTO содержит `id`, `slug`, `name`, `description`, `visibility`, `status`, `created_at`, `updated_at`, `icon_url`. Даты — Unix seconds UTC. `icon_url` — null либо путь от корня `/api/v1/admin/applications/{id}/icon?v={opaque-version}`. Путь хранения и внутренний icon_key не входят в DTO. Публичность private/public и статус active/archived — независимые свойства.

## Иконки

POST `/{id}/icon` принимает **сырое тело файла**, без multipart и без JSON/base64. Клиент может указать Content-Type image/png, image/jpeg или image/webp; сервер определяет формат по реальному содержимому. Исходное имя не передаётся и не используется. Максимум 2 МиБ (2097152 байта), ширина и высота 1–1024 пикселя. SVG, GIF, повреждённые изображения и превышение лимитов — 400 invalid_icon. При ошибке прежняя иконка остаётся действующей. Сервер декодирует изображение и сохраняет PNG, у анимации — первый кадр.

GET иконки защищён той же сессией и доступен оператору. Все API-ответы, включая изображение, имеют Cache-Control: no-store; замена создаёт новый version URL. Произвольный путь или ключ хранения не принимается. Иконки не публикуются через static assets.

## Ошибки

Общий envelope: `{"error":{"code":"…","message":"…"}}`. Внутренние причины и пути клиенту не передаются.

| HTTP | Код | Причина |
| --- | --- | --- |
| 400 | invalid_application | поля, JSON или query не прошли валидацию |
| 400 | invalid_icon | формат, содержимое, размер или пиксели иконки |
| 400 | invalid_request | некорректное кодирование параметра пути |
| 401 | unauthorized | отсутствующая, отозванная или истёкшая сессия; заблокированный пользователь |
| 403 | forbidden | нет роли admin для изменения либо не пройдена защита Origin/header/CSRF |
| 404 | not_found | отсутствующий UUID приложения или иконка |
| 405 | method_not_allowed | неподдерживаемый метод |
| 409 | slug_exists | slug занят, включая архив или конкурентное создание |
| 500 | internal_error | внутренняя ошибка |

Публичных страниц, скачиваний, манифестов, API релизов и прав на отдельные приложения на этапе 3 нет.

## Публикации и обновления (этап 5)

Базовый путь для таблицы — `/api/v1/admin/applications/{applicationId}`. Все запросы требуют живую session cookie. Каналы/историю и Tauri preview читают admin/operator; validation публикации, preview универсального манифеста, публикацию, переключение и updater metadata — только admin. Все POST/PUT/DELETE проходят существующие Origin/custom-header/CSRF проверки, изменения повторно проверяют session/admin в write-транзакции.

| Метод | Суффикс | Тело / результат |
| --- | --- | --- |
| GET | `/channels` | массив Channel DTO |
| POST | `/releases/{releaseId}/validate-publication` | `{channel}` → `{release_revision,channel,validation}` |
| POST | `/releases/{releaseId}/publish` | `{channel,expected_channel_revision,expected_release_revision}` → Channel DTO |
| PUT | `/channels/{channel}/current` | `{release_id,expected_channel_revision,expected_release_revision}` → Channel DTO |
| GET | `/publication-history?limit=50&before={eventId}` | массив событий; limit 1–100, cursor исключительный |
| GET | `/manifests/{channel}/preview?limit=20` | универсальный манифест; limit 1–100 |
| POST | `/updater/tauri/preview` | `{channel,target,current_version}` → диагностический TauriPreview |
| PUT | `/releases/{releaseId}/artifacts/{artifactId}/updater` | `{distribution_role:"installer" или "updater"}` → Artifact DTO |
| PUT | `/releases/{releaseId}/artifacts/{artifactId}/signature` | сырое содержимое `.sig`, до 16384 байт → Artifact DTO |
| DELETE | `/releases/{releaseId}/artifacts/{artifactId}/signature` | очистить подпись → Artifact DTO |

Публикация draft в канал и переключение current атомарны с history. Publish также может назначить уже опубликованный релиз в другой канал, не меняя опубликованные метаданные; обычный UI использует для этого переключение. Нельзя опубликовать/переключить в архивном приложении или назначить prerelease в stable. Переключение требует опубликованный релиз. Любой UUID чужого приложения даёт 404.

Channel DTO: `id,application_id,name,current_release_id,current_version,published_at,release_count,revision,created_at,updated_at`. Пустой канал содержит NULL current/published_at и count=0. Release DTO расширен `revision,channels`, status допускает published. Artifact DTO расширен `distribution_role,signature_present`; содержимое подписи, storage_key и путь не возвращаются. GET списка релизов поддерживает необязательный `status=draft|published`.

Validation: `state=ready|warnings|errors`, массивы `warnings` и `errors`; каждый issue содержит `code,artifact_id,message`. Наличие errors запрещает публикацию. В validation API published релиз получает `release_published`; этот endpoint проверяет готовность нового draft. Signature missing у явно назначенного updater — warning. Обязательного набора целевых платформ у универсального приложения нет.

При публикации используйте revision из последней проверки, а не timestamp:

```json
{"channel":"stable","expected_channel_revision":3,"expected_release_revision":6}
```

409 `publication_conflict` требует обновить данные и пересмотреть подтверждение. Завершённый повтор, когда тот же published уже current, идемпотентен: нет новой истории/revision. 422 `publication_blocked` возвращает общий error envelope и дополнительное поле `validation` с конкретными проблемами. Проверка files/SHA-256 выполняется заново, даже если preflight успешно завершился ранее. Недоступное storage из-за занятости возвращает 503 `storage_busy`.

History DTO: `id,application_id,release_id,channel,action,previous_release_id,previous_version,new_release_id,new_version,user_id,username,created_at`. Порядок id DESC; для следующей страницы передайте последний id как before. Изменять историю через API нельзя.

Подпись можно прикрепить только draft artifact с ролью updater; повторный PUT заменяет её, DELETE очищает. Переключение роли на installer удаляет подпись. При несовместимых target/format, неверной структуре подписи, пути/URL/key-файле — 422 `invalid_updater`. Превышение сырого тела — 413. Проверка структуры `.sig` не заменяет криптографическую проверку клиентом. Все metadata/подписи опубликованного файла защищены 409 `release_locked`.

TauriPreview: `state,reason,message,selected_version,artifact_id,artifact_filename,signature_present,public_downloads_available,response`. При успешном выборе подписанного публичного пакета reason=update_available, state=available, response содержит Tauri JSON и public_downloads_available=true. При той же/старшей версии клиента — version_not_newer. Другие причины и модель описаны в [управлении релизами](release-management.md). Этот административный endpoint не является production updater; он не отдаёт протокольный 204/200 и не публикуется клиентам.

Все административные ответы — no-store. Некорректные JSON/unknown fields/query — 400, неверный SemVer — 422, несуществующие ресурсы — 404, middleware gates — 401/403. Новые JSON/подписи ограничены 16 КиБ; входные имена/версии дополнительно ограничены доменными правилами. При 422 publication_blocked клиент показывает validation из ответа. Полный публичный контракт `GET /{slug}/manifest.json`, ETag и 404 для private/archived описаны в [release-management.md](release-management.md#универсальный-манифест).


## Распространение (этап 6)

База `/api/v1/admin/applications/{applicationId}`; все ответы no-store.

| Метод | Суффикс | Роль / результат |
| --- | --- | --- |
| GET | `/distribution` | admin/operator: публичные URL, public_available, channels[{name,version}], default_distribution |
| PUT | `/distribution/default` | admin + CSRF: `{target,format,purpose:"installer"}` либо `null` для сброса; 204 |
| GET | `/distribution/releases/{releaseId}` | admin/operator: public_available, files[{id,url,tauri_compatible,signature_present}], warnings[] |

Селектор валидируется доменом, сессия/роль перепроверяются внутри write transaction. Неизвестные поля — 400, неверный target/format/purpose — 422, session/CSRF/role — 401/403. Селектор не связан с изменением опубликованных файлов; неизвестный UUID/чужой release — 404. default_distribution_missing появляется как warning при проверке публикации. Публичные ссылки private/archived показываются как настройки с предупреждением о недоступности; private preview манифеста не включает download_url. Public API и HTTP Range: [public-distribution.md](public-distribution.md). Updater: [integrations/tauri.md](integrations/tauri.md).

## Настройка Tauri-интеграции

Описание GET/PUT `/applications/{id}/integrations/tauri`, явного POST `/readiness`, полей, прав и примеров: [Tauri — административный API](integrations/tauri.md#статусы-и-административный-api). Префикс `/api/v1/admin` и session/CSRF сохраняются. Оператор может читать настройки и вызывать read-only preview; изменение публичного ключа требует admin с повторной проверкой сессии в транзакции. Публичные ключи не секретны; приватные ключи и пароли не принимаются.
