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
| GET | `/{id}/integrations/standalone` | 200, `{application_id,public_key}`; operator/admin |
| PUT | `/{id}/integrations/standalone` | 204; `{public_key: string|null}`; admin + CSRF |
| PUT | `/{id}/releases/{release}/artifacts/{artifact}/standalone` | 200, Artifact DTO; `{signed_metadata: string|null}`; только черновик, admin + CSRF |

`id` — постоянный UUID. Физическое удаление приложений отсутствует; DELETE `/{id}` возвращает 405.

Multipart загрузка артефакта также принимает файл `standalone_signature` (`.jws`, максимум 4096 байт), проверяемый до фиксации. Наличие подписи явно назначает installer для standalone-обновлений. Artifact DTO дополнен `standalone_signature_present`; роль и прежний `signature_present` для Tauri не меняются. Опубликованный релиз можно дополнить подписанным файлом другой платформы в одной загрузке. Формат JWS, ограничения ключа и примеры: [standalone](integrations/standalone.md).

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

## Загрузка в существующий релиз

`POST /api/v1/admin/applications/{id}/releases/{releaseId}/artifacts` доступен admin для draft/published с прежними session/Origin/CSRF-проверками. Multipart: target, необязательные file_type/distribution_role до file, необязательный файловый part signature (.sig, до 16 КиБ) до или после file. Отсутствующая distribution_role означает installer. Детали и curl: [загрузка файлов](releases.md#административный-api).

Уникальны target/format/role внутри релиза, updater допускается один на target независимо от формата. Конфликт загрузки или редактирования роли draft — 409 artifact_exists. Published updater без подписи в том же запросе — 422 updater_signature_required; некорректная подпись/совместимость — 422 invalid_updater. Несколько форматов установщика разрешены; неоднозначность проверяется при скачивании. Все ошибки сохраняют прежние записи, revision и публикацию.

Успех: 201 Artifact DTO, атомарно увеличенный release.revision. Первоначальный published_at, updated_at, версия и история не меняются. Изменение/удаление старых published артефактов, включая подпись и роль, остаётся 409 release_locked. Повторная загрузка не заменяет прежний UUID.

## Публикации, манифесты и распространение

База `/api/v1/admin/applications/{id}`. Живая сессия обязательна; все mutations требуют Origin/custom-header/CSRF. Публикация, изменение ролей и подписей требуют admin и повторной проверки сессии в транзакции. Оператор читает историю, ссылки распространения и Tauri preview. Preview универсального манифеста требует admin.

| Метод | Суффикс | Тело / результат |
| --- | --- | --- |
| POST | `/releases/{release}/validate-publication` | `{}` → `{release_revision,validation}` |
| POST | `/releases/{release}/publish` | `{expected_release_revision}` → Release DTO |
| GET | `/publication-history?limit=50&before={id}` | массив фактов публикации, limit 1–100 |
| GET | `/manifests/preview?limit=20` | манифест schema_version=2, limit 1–100 |
| POST | `/updater/tauri/preview` | `{target,current_version}` → TauriPreview |
| PUT | `/releases/{release}/artifacts/{artifact}/updater` | `{distribution_role:"installer" или "updater"}` → Artifact DTO |
| PUT | `/releases/{release}/artifacts/{artifact}/signature` | содержимое `.sig`, до 16384 байт → Artifact DTO |
| DELETE | `/releases/{release}/artifacts/{artifact}/signature` | очистить подпись черновика |
| GET | `/distribution` | public_available, application_url, latest_url, manifest_url, tauri_url, downloads |
| GET | `/distribution/releases/{release}` | public_available, files с постоянными URL, warnings |

Validation: state=ready|warnings|errors, warnings/errors с code,artifact_id,message. Для публикации используйте revision из проверки. Сервер заново проверяет файлы и snapshot; конфликт — 409 publication_conflict, ошибки готовности — 422 publication_blocked с validation. Завершённый повтор идемпотентен. Установленный published защищён от редактирования/удаления; разрешено добавление новых неизменяемых файлов.

History DTO: id,application_id,release_id,version,user_id,username,created_at; id DESC, cursor before исключительный. `downloads[]`: target,version,formats,url,format_urls[{format,url}]. URL платформы при нескольких форматах требует уточнения format. Private/archived показывают ссылки с public_available=false, private preview не включает download_url.

TauriPreview: state,reason,message,selected_version,artifact_id,artifact_filename,signature_present,public_downloads_available,response. `available` содержит реальный Tauri JSON, `no_update` объясняет отсутствие новой версии/пакета, `blocked` — ошибку файла/подписи/публичности. Алгоритм тот же, что у публичного endpoint; этот диагностический API не предназначен для production-клиента.

Подпись/роль меняются только у draft; published — 409 release_locked. Неверная структура `.sig`/совместимость — 422 invalid_updater, слишком большое тело — 413. Серверная проверка envelope не заменяет криптографическую проверку подписи клиентом. Все ответы admin — no-store; неизвестные поля/query — 400, session/role/CSRF — 401/403, чужие UUID — 404. [Публичное API](public-distribution.md), [правила публикации](release-management.md).

## Настройка Tauri-интеграции

Описание GET/PUT `/applications/{id}/integrations/tauri`, явного POST `/readiness`, полей, прав и примеров: [Tauri — административный API](integrations/tauri.md#статусы-и-административный-api). Префикс `/api/v1/admin` и session/CSRF сохраняются. Оператор может читать настройки и вызывать read-only preview; изменение публичного ключа требует admin с повторной проверкой сессии в транзакции. Публичные ключи не секретны; приватные ключи и пароли не принимаются.
