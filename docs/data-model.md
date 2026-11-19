# Модель данных

SQLite с foreign_keys=ON на каждом соединении, WAL, bounded pool и busy timeout. Все миграции добавляются последовательно; применённые SQL не изменяются. Время в SQLite — INTEGER Unix seconds UTC. В манифестах — UTC RFC 3339, в административных DTO — Unix seconds.

Существующие identity/application таблицы описаны в [архитектуре](architecture.md). Миграция 0005 перестраивает releases/artifacts с переносом строк при включённых внешних ключах: старые artifacts удаляются прежде старого parent, новые child-ссылки корректируются SQLite при переименовании нового parent. Ключи дисковых объектов не меняются. Проверка миграции покрывает реальные файлы/хеши, FK check и повторный startup.

## releases

UUID id; application_id → applications, RESTRICT; version и version_key (без build metadata); title; release_notes; status `draft|published`; revision; created_at; updated_at; published_at. UNIQUE(application_id,version_key), UNIQUE(id,application_id) для составных FK. CHECK связывает status с NULL/значением published_at. Новые релизы — draft. При metadata/артефактных изменениях сервис увеличивает revision; timestamp не используется для optimistic locking.

Идентичность id/application_id/created_at неизменна. При OLD.status=published триггеры запрещают удаление и изменение полей, кроме технического revision=revision+1. updated_at и первоначальный published_at сохраняются при дополнении файлами. Первичная публикация изменяет только status/published_at/updated_at. Физического удаления опубликованных объектов нет.

## artifacts

UUID id; release_id → releases, CASCADE для удаляемых черновиков; filename (метаданные); target; file_type; size_bytes>0; sha256 (64 lowercase hex); уникальный непрозрачный storage_key; created_at; distribution_role `installer|updater` (default installer); nullable updater_signature (до 16384 символов, только updater).

UNIQUE(release_id,target,file_type,distribution_role). Частичный UNIQUE(release_id,target) WHERE distribution_role='updater' разрешает максимум один updater на платформу независимо от формата. Базовые поля неизменны даже у draft; заменяют файл удалением/новой загрузкой. У draft сервис позволяет менять роль/подпись с увеличением revision родителя. У published разрешён INSERT нового файла; UPDATE/DELETE существующих запрещены. Отдельный BEFORE INSERT не допускает замену опубликованного объекта через INSERT OR REPLACE, а новый published updater требует непустую подпись. Структура подписи и совместимость формата проверяются сервисом. Диск обслуживается только через ArtifactStorage. DB-транзакция не считается атомарной с файловой системой.

## release_channels

UUID id; application_id → applications RESTRICT; name (ASCII lowercase, цифры и дефис, 1–64); nullable current_release_id; revision>=0; created_at; updated_at. UNIQUE(application_id,name), UNIQUE(id,application_id). Составной FK(current_release_id,application_id) → releases(id,application_id) запрещает чужое приложение; FK(id,current_release_id) → channel_releases(channel_id,release_id) требует membership. Идентичность канала неизменна.

Имя — данные, без enum stable/beta в CHECK. Миграция создаёт две default-записи для всех приложений; AFTER INSERT applications создаёт их для новых в той же транзакции. UUID генерируются SQLite randomblob, без производных ключей или collisions от усечения application UUID. Изменения указателя выполняются с expected revision и её увеличением. Список доступных каналов получает UI из API; API создания иных каналов пока отсутствует.

## channel_releases

PRIMARY KEY(channel_id,release_id); application_id; added_at. Составные FK на channel и release в одном приложении. Триггер допускает только published. UPDATE/DELETE запрещены: membership сохраняет доступность истории канала, не копируя файлы. Один release может участвовать в нескольких каналах.

## publication_history

Монотонный INTEGER id; application_id; channel_id; release_id; action; nullable previous_release_id; new_release_id; user_id; created_at. Типы action: release_published/channel_promoted/channel_rolled_back. Ссылки канала и релизов — составные по application_id; пользователь сохраняется по FK RESTRICT. Индекс(application_id,id DESC) обслуживает курсор истории. UPDATE/DELETE запрещены триггерами. Вставка входит в одну транзакцию с публикацией/current, отдельной системы общего аудита нет.

## Расширение

ApiToken и DownloadEvent не создаются. Публичные download URL не записываются в artifacts на этапе 5. Универсальный манифест и Tauri получают один current/membership; независимо сохранённого JSON нет. Другие протоколы и пользовательские каналы добавляются вместе с их сценариями.


## Настройка распространения (этап 6)

Миграция 0006 добавляет application_distribution: application_id PK/FK, target, format, purpose (CHECK installer). Отсутствующая строка означает автоматический выбор только при единственном installer. Значения проверяются доменом и DB CHECK по длине/назначению. Настройка изменяема независимо от published Release/Artifact; UUID/slug, membership, immutability и append-only история не меняются.

## Публичный ключ Tauri

Миграция `0007_tauri_integration.sql` добавляет `applications.tauri_public_key TEXT NULL` (1–4096 символов при наличии). Сервис принимает только публичный Tauri CLI/minisign envelope; `NULL` означает отсутствие настройки. Ключ используется для генерируемых инструкций, не для серверного подписания или объявления криптографического успеха. Существующие приложения после миграции получают `NULL`; подписи артефактов и инварианты публикации не изменены.

## Дополнение релиза

Миграция `0008_artifact_roles_and_append.sql` перестраивает только artifacts с включёнными FK, переносит все колонки без изменения UUID, ключей, SHA-256 и подписей. Releases, каналы и история не перестраиваются. При прежних нескольких updater на одном target миграция атомарно отклоняется: автоматическое удаление/переназначение опубликованных файлов недопустимо. Перед обновлением проверьте `SELECT release_id,target,count(*) FROM artifacts WHERE distribution_role='updater' GROUP BY release_id,target HAVING count(*)>1`. Для черновика исправьте назначения старой версией сервера; конфликт опубликованных данных требует отдельного согласованного решения с сохранением исходной БД и файлов.

Добавление файла выполняется через ArtifactStorage: потоковый SHA-256, полное сохранение объекта, затем короткая BEGIN IMMEDIATE с повторной авторизацией, проверкой статуса, ограничений и текущего default selector. Запись артефакта и revision родителя фиксируются вместе; membership/current/history и published_at остаются прежними. Незавершённый или отклонённый объект компенсируется существующим guard; неоднозначный commit оставляет безопасный файл до startup reconciliation. Манифест читает current и все списки файлов одним read snapshot, ETag рассчитывается из его JSON.
