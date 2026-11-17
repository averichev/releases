# Модель данных

SQLite с foreign_keys=ON на каждом соединении, WAL, bounded pool и busy timeout. Все миграции добавляются последовательно; применённые SQL не изменяются. Время в SQLite — INTEGER Unix seconds UTC. В манифестах — UTC RFC 3339, в административных DTO — Unix seconds.

Существующие identity/application таблицы описаны в [архитектуре](architecture.md). Миграция 0005 перестраивает releases/artifacts с переносом строк при включённых внешних ключах: старые artifacts удаляются прежде старого parent, новые child-ссылки корректируются SQLite при переименовании нового parent. Ключи дисковых объектов не меняются. Проверка миграции покрывает реальные файлы/хеши, FK check и повторный startup.

## releases

UUID id; application_id → applications, RESTRICT; version и version_key (без build metadata); title; release_notes; status `draft|published`; revision; created_at; updated_at; published_at. UNIQUE(application_id,version_key), UNIQUE(id,application_id) для составных FK. CHECK связывает status с NULL/значением published_at. Новые релизы — draft. При metadata/артефактных изменениях сервис увеличивает revision; timestamp не используется для optimistic locking.

Идентичность id/application_id/created_at неизменна. BEFORE UPDATE/DELETE при OLD.status=published запрещают любое изменение и удаление. Первичная публикация изменяет только status/published_at/updated_at. Физического удаления опубликованных объектов нет.

## artifacts

UUID id; release_id → releases, CASCADE для удаляемых черновиков; filename (метаданные); target; file_type; size_bytes>0; sha256 (64 lowercase hex); уникальный непрозрачный storage_key; created_at; distribution_role `installer|updater` (default installer); nullable updater_signature (до 16384 символов, только updater).

UNIQUE(release_id,target,file_type). Базовые поля неизменны даже у draft; заменяют файл удалением/новой загрузкой. У draft сервис позволяет менять роль/подпись с увеличением revision родителя. BEFORE INSERT/UPDATE/DELETE на published parent запрещают изменение состава и updater metadata. Диск обслуживается только через ArtifactStorage. DB-транзакция не считается атомарной с файловой системой.

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
