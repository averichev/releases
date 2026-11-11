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
