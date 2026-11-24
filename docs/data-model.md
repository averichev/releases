# Модель данных

Миграция `0010_standalone_updates.sql` добавляет `applications.standalone_public_key` (nullable, уникальный публичный Ed25519 JWK x) и `artifacts.standalone_signature` (nullable compact JWS, до 4096 байт). Подпись разрешена только для installer формата exe/bin с конкретными ОС/архитектурой. Один standalone на release/target; ключ нельзя заменить при наличии подписанных артефактов. Существующие триггеры неизменности опубликованных файлов распространяются на новое поле. Отдельный триггер закрывает обход через REPLACE. Исторические строки получают NULL и продолжают работать без изменений. [Контракт и доверие](integrations/standalone.md).

SQLite с foreign_keys=ON на каждом соединении, WAL, bounded pool и busy timeout. Все миграции добавляются последовательно; применённые SQL не изменяются. Время в SQLite — INTEGER Unix seconds UTC. В манифестах — UTC RFC 3339, в административных DTO — Unix seconds.

Существующие identity/application таблицы описаны в [архитектуре](architecture.md). Миграция 0005 перестраивает releases/artifacts с переносом строк при включённых внешних ключах: старые artifacts удаляются прежде старого parent, новые child-ссылки корректируются SQLite при переименовании нового parent. Ключи дисковых объектов не меняются. Проверка миграции покрывает реальные файлы/хеши, FK check и повторный startup.

## Идентичность standalone-продукта

`applications.id` — внутренний неизменяемый UUID для FK и административного API; `slug` — постоянный адрес в этом экземпляре сервера. Миграция `0011_standalone_product_identity.sql` добавляет nullable `applications.product_id` — UUID издателя в канонической строчной форме `8-4-4-4-12`, с CHECK и частичным UNIQUE для всех приложений, включая архив. При создании записи он NULL: генерации на сервере нет. Один UUID допустим на разных независимых серверах. Поле `application_id` в JWS v1 хранит этот `product_id`; дополнительных полей протокола нет.

`product_id_locked` — INTEGER 0/1 (по умолчанию 0), техническое постоянное закрепление после первой standalone-подписи. INSERT/UPDATE подписанного артефакта требует непустой `product_id` и атомарно ставит флаг. Триггеры запрещают изменение/очистку закреплённого UUID, снятие флага и замену такой записи через INSERT OR REPLACE. Удаление подписанного черновика не снимает закрепление. Связи release/application и artifact/release остаются внутренними UUID.

Миграция назначает `product_id=id`, `product_id_locked=1` только существующим приложениям с `standalone_signature IS NOT NULL` в любом релизе, включая draft. Остальные остаются NULL/0, даже при настроенном ключе. Содержимое JWS, UUID релизов/артефактов, ключи, версии, файлы и их SHA-256 сохраняются. Старые подписи проверяются побайтно без нового подписания.

Миграция `0010_standalone_updates.sql` хранит nullable `applications.standalone_public_key` (публичный Ed25519 JWK x, 43 base64url символа) и `artifacts.standalone_signature` (1–4096 символов JWS, только совместимый installer). Ключ уникален внутри сервера и защищён от замены/очистки при наличии подписей, включая черновики. Частичный UNIQUE(release_id,target) при непустой standalone_signature исключает неоднозначный standalone. Приватные ключи в БД отсутствуют.

## releases

UUID id; application_id → applications, RESTRICT; version и version_key (без build metadata); title; release_notes; status `draft|published`; revision; created_at; updated_at; published_at. UNIQUE(application_id,version_key), UNIQUE(id,application_id) для составных FK. CHECK связывает status с NULL/значением published_at. Новые релизы — draft. При metadata/артефактных изменениях сервис увеличивает revision; timestamp не используется для optimistic locking.

Идентичность id/application_id/created_at неизменна. При OLD.status=published триггеры запрещают удаление и изменение полей, кроме технического revision=revision+1. updated_at и первоначальный published_at сохраняются при дополнении файлами. Первичная публикация изменяет status/published_at/updated_at и увеличивает revision. Физического удаления опубликованных объектов нет.

## artifacts

UUID id; release_id → releases, CASCADE для удаляемых черновиков; filename (метаданные); target; file_type; size_bytes>0; sha256 (64 lowercase hex); уникальный непрозрачный storage_key; created_at; distribution_role `executable|installer|updater|archive` (default installer); nullable updater_signature (до 16384 символов, только updater).

UNIQUE(release_id,target,file_type,distribution_role). Частичный UNIQUE(release_id,target) WHERE distribution_role='updater' разрешает максимум один updater на платформу независимо от формата. Базовые поля неизменны даже у draft; заменяют файл удалением/новой загрузкой. У draft сервис позволяет менять роль/подпись с увеличением revision родителя. У published разрешён INSERT нового файла; UPDATE/DELETE существующих запрещены. Отдельный BEFORE INSERT не допускает замену опубликованного объекта через INSERT OR REPLACE, а новый published updater требует непустую подпись. Структура подписи и совместимость формата проверяются сервисом. Диск обслуживается только через ArtifactStorage. DB-транзакция не считается атомарной с файловой системой.

## publication_history

Монотонный INTEGER id; application_id; release_id UNIQUE; nullable user_id/token_id; created_at. CHECK требует ровно одного автора. Составные FK(release_id,application_id) и FK(token_id,application_id) ссылаются на релиз и токен того же приложения; авторы защищены FK RESTRICT. Индекс(application_id,id DESC) обслуживает cursor before. UPDATE/DELETE запрещены. Факт первой публикации записывается атомарно со статусом релиза; дополнение файлами не создаёт новую публикацию. Миграция 0016 сохраняет все прежние user_id, ID событий и даты.

DownloadEvent не создаётся. Публичные URL не сохраняются в БД. Каталог опубликованных версий читается в одном snapshot и сортируется по SemVer. Выбор установщиков выполняется отдельно для target/формата, подписанных updater — отдельно по совместимости. Limit истории не ограничивает поиск актуальных платформ.

## api_tokens и api_publications

Миграция 0016 добавляет ApiToken: UUID, application_id, индексируемый UNIQUE token_hash (SHA-256 всего секрета с префиксом), name, created_at/created_by, nullable expires_at и пару revoked_at/revoked_by. Идентичность и параметры выдачи неизменяемы; отзыв однократный, удаление запрещено. Триггер ограничивает число действующих токенов приложения двумя, включая конкурентную выдачу. В списке нет hash или секрета. Генерация использует системные 32 случайных байта из infrastructure/secrets.

`api_publications(release_id PRIMARY KEY, manifest TEXT)` хранит нормализованный исходный JSON только для выпусков нового endpoint: полную версию, notes и отсортированные описания фактических файлов/принятых подписей. Не содержит токена, transport part names, boundary, URL и путей. INSERT разрешён только для published; UPDATE/DELETE запрещены. Manifest создаётся в одной транзакции с релизом и событием. Уникальность версии остаётся прежней `(application_id,version_key)`; ручные версии не получают API manifest и всегда конфликтуют с CLI-публикацией.

## artifact_download_counts

Миграция `0013_download_counts.sql` добавляет `artifact_id` — PRIMARY KEY/FK → artifacts с CASCADE и `download_count` — INTEGER NOT NULL с CHECK целочисленности и неотрицательности. До первого скачивания строки нет, читается ноль; исторические загрузки не восстанавливаются. Опубликованные артефакты и версии защищены от удаления существующими триггерами, поэтому их итоги сохраняются и после архивации/скрытия приложения.

Один INSERT SELECT ON CONFLICT атомарно проверяет принадлежность artifact → release → application, published/public/active и увеличивает счётчик. Параллельные запросы не теряют приращения. Версия и приложение получают SUM по связям UUID: дублирующих итоговых счётчиков нет. Изменение счётчика не меняет revision, timestamps, содержимое подписанных артефактов или ETag манифеста. Индексы artifacts(release_id), releases(application_id) и PRIMARY KEY счётчика обслуживают агрегации. Журнал событий, IP, cookie, User-Agent и персональные идентификаторы не сохраняются.

Переход существующей базы и проверка ограничения публичного доступа: [миграция 0009](migration-0009.md). В актуальной схеме остаются Application, Release, Artifact и история фактических публикаций; отдельного селектора установщика приложения нет.

## Публичный ключ Tauri

Миграция `0007_tauri_integration.sql` добавляет `applications.tauri_public_key TEXT NULL` (1–4096 символов при наличии). Сервис принимает только публичный Tauri CLI/minisign envelope; `NULL` означает отсутствие настройки. Ключ используется для генерируемых инструкций, не для серверного подписания или объявления криптографического успеха. Существующие приложения после миграции получают `NULL`; подписи артефактов и инварианты публикации не изменены.

## Дополнение релиза

Миграция `0008_artifact_roles_and_append.sql` перестраивает только artifacts с включёнными FK, переносит все колонки без изменения UUID, ключей, SHA-256 и подписей. Releases и история не перестраиваются. При прежних нескольких updater на одном target миграция атомарно отклоняется: автоматическое удаление/переназначение опубликованных файлов недопустимо. Перед обновлением проверьте `SELECT release_id,target,count(*) FROM artifacts WHERE distribution_role='updater' GROUP BY release_id,target HAVING count(*)>1`. Для черновика исправьте назначения старой версией сервера; конфликт опубликованных данных требует отдельного согласованного решения с сохранением исходной БД и файлов.

Добавление файла выполняется через ArtifactStorage: потоковый SHA-256, полное сохранение объекта, затем короткая BEGIN IMMEDIATE с повторной авторизацией, проверкой статуса, ограничений уникальности. Запись артефакта и revision родителя фиксируются вместе; history и published_at остаются прежними. Незавершённый или отклонённый объект компенсируется существующим guard; неоднозначный commit оставляет безопасный файл до startup reconciliation. Манифест читает опубликованные версии и списки файлов одним read snapshot, ETag рассчитывается из его JSON.

## Универсальные типы артефактов

Миграция `0012_artifact_types.sql` расширяет CHECK существующего artifacts.distribution_role, не создавая параллельного поля. `installer` и `updater` сохраняют значения; новые типы — `executable` и `archive`. Исторические installer не переопределяются по имени: назначение EXE невозможно достоверно восстановить из расширения. Опубликованные файлы остаются неизменяемыми.

Перестраивается только artifacts при включённых FK. Все UUID, release_id, filename, target, file_type, размеры, SHA-256, storage_key, timestamps и обе подписи копируются без изменения; физические файлы не затрагиваются. Восстанавливаются индексы, запрет UPDATE/DELETE опубликованных объектов, защита от REPLACE, требования Tauri и существующие standalone guards. Уникальность остаётся `(release_id,target,file_type,distribution_role)` плюс один Tauri updater на release/target. Поэтому Windows EXE executable и installer одной версии не конфликтуют, а два executable одного target/формата отклоняются.

Классификация executable не включает автоматически существующий механизм подписанных standalone-обновлений. Старые standalone installer и их JWS/ключи сохраняются; обычный executable публикуется без подписи и ключей. PublicDistribution фильтрует latest по target/типу/формату до выбора SemVer; старый installer latest и latest_by_target совместимы. Каталог и manifest v2 дополнены latest_by_type для ручного скачивания executable/installer/archive.
