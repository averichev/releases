# Модель данных

SQLite с foreign_keys=ON на каждом соединении, WAL, bounded pool и busy timeout. Все миграции добавляются последовательно; применённые SQL не изменяются. Время в SQLite — INTEGER Unix seconds UTC. В манифестах — UTC RFC 3339, в административных DTO — Unix seconds.

Существующие identity/application таблицы описаны в [архитектуре](architecture.md). Миграция 0005 перестраивает releases/artifacts с переносом строк при включённых внешних ключах: старые artifacts удаляются прежде старого parent, новые child-ссылки корректируются SQLite при переименовании нового parent. Ключи дисковых объектов не меняются. Проверка миграции покрывает реальные файлы/хеши, FK check и повторный startup.

## releases

UUID id; application_id → applications, RESTRICT; version и version_key (без build metadata); title; release_notes; status `draft|published`; revision; created_at; updated_at; published_at. UNIQUE(application_id,version_key), UNIQUE(id,application_id) для составных FK. CHECK связывает status с NULL/значением published_at. Новые релизы — draft. При metadata/артефактных изменениях сервис увеличивает revision; timestamp не используется для optimistic locking.

Идентичность id/application_id/created_at неизменна. При OLD.status=published триггеры запрещают удаление и изменение полей, кроме технического revision=revision+1. updated_at и первоначальный published_at сохраняются при дополнении файлами. Первичная публикация изменяет status/published_at/updated_at и увеличивает revision. Физического удаления опубликованных объектов нет.

## artifacts

UUID id; release_id → releases, CASCADE для удаляемых черновиков; filename (метаданные); target; file_type; size_bytes>0; sha256 (64 lowercase hex); уникальный непрозрачный storage_key; created_at; distribution_role `installer|updater` (default installer); nullable updater_signature (до 16384 символов, только updater).

UNIQUE(release_id,target,file_type,distribution_role). Частичный UNIQUE(release_id,target) WHERE distribution_role='updater' разрешает максимум один updater на платформу независимо от формата. Базовые поля неизменны даже у draft; заменяют файл удалением/новой загрузкой. У draft сервис позволяет менять роль/подпись с увеличением revision родителя. У published разрешён INSERT нового файла; UPDATE/DELETE существующих запрещены. Отдельный BEFORE INSERT не допускает замену опубликованного объекта через INSERT OR REPLACE, а новый published updater требует непустую подпись. Структура подписи и совместимость формата проверяются сервисом. Диск обслуживается только через ArtifactStorage. DB-транзакция не считается атомарной с файловой системой.

## publication_history

Монотонный INTEGER id; application_id; release_id UNIQUE; user_id; created_at. Составной FK(release_id,application_id) ссылается на релиз того же приложения; пользователь защищён FK RESTRICT. Индекс(application_id,id DESC) обслуживает cursor before. UPDATE/DELETE запрещены. Факт первой публикации записывается атомарно со статусом релиза; дополнение файлами не создаёт новую публикацию.

ApiToken и DownloadEvent не создаются. URL и JSON не сохраняются в БД. Каталог опубликованных версий читается в одном snapshot и сортируется по SemVer. Выбор установщиков выполняется отдельно для target/формата, подписанных updater — отдельно по совместимости. Limit истории не ограничивает поиск актуальных платформ.

Переход существующей базы и проверка ограничения публичного доступа: [миграция 0009](migration-0009.md). В актуальной схеме остаются Application, Release, Artifact и история фактических публикаций; отдельного селектора установщика приложения нет.

## Публичный ключ Tauri

Миграция `0007_tauri_integration.sql` добавляет `applications.tauri_public_key TEXT NULL` (1–4096 символов при наличии). Сервис принимает только публичный Tauri CLI/minisign envelope; `NULL` означает отсутствие настройки. Ключ используется для генерируемых инструкций, не для серверного подписания или объявления криптографического успеха. Существующие приложения после миграции получают `NULL`; подписи артефактов и инварианты публикации не изменены.

## Дополнение релиза

Миграция `0008_artifact_roles_and_append.sql` перестраивает только artifacts с включёнными FK, переносит все колонки без изменения UUID, ключей, SHA-256 и подписей. Releases и история не перестраиваются. При прежних нескольких updater на одном target миграция атомарно отклоняется: автоматическое удаление/переназначение опубликованных файлов недопустимо. Перед обновлением проверьте `SELECT release_id,target,count(*) FROM artifacts WHERE distribution_role='updater' GROUP BY release_id,target HAVING count(*)>1`. Для черновика исправьте назначения старой версией сервера; конфликт опубликованных данных требует отдельного согласованного решения с сохранением исходной БД и файлов.

Добавление файла выполняется через ArtifactStorage: потоковый SHA-256, полное сохранение объекта, затем короткая BEGIN IMMEDIATE с повторной авторизацией, проверкой статуса, ограничений уникальности. Запись артефакта и revision родителя фиксируются вместе; history и published_at остаются прежними. Незавершённый или отклонённый объект компенсируется существующим guard; неоднозначный commit оставляет безопасный файл до startup reconciliation. Манифест читает опубликованные версии и списки файлов одним read snapshot, ETag рассчитывается из его JSON.
