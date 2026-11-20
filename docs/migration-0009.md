# Переходная совместимость: миграция 0009

Этот документ и соответствующие fixtures в tests/infrastructure.rs описывают только переход со старой модели каналов. Рабочие API и интерфейс каналов не имеют.

Миграция `0009_remove_channels.sql` удаляет release_channels, channel_releases, application_distribution. Она сохраняет releases/artifacts без перестройки: UUID, SemVer/build metadata, даты, revision, SHA-256, storage keys и подписи не меняются. Файлы не перемещаются и не удаляются. Publication_history переносит факты первой публикации (`release_published`) с прежними id, пользователем и временем. События переключения/отката указателя не являются публикациями и удаляются вместе с моделью указателей. Для полного старого аудита сохраните backup.

Перед обновлением остановите запись и сделайте согласованную резервную копию SQLite и файлов. Не запускайте новую версию на единственной копии до проверки восстановления. Выполните на старой базе:

```sql
SELECT a.slug, r.id, r.version, a.visibility, a.status
FROM releases r JOIN applications a ON a.id=r.application_id
WHERE r.status='published' AND NOT EXISTS (
  SELECT 1 FROM channel_releases cr JOIN release_channels c ON c.id=cr.channel_id
  WHERE cr.release_id=r.id AND c.name='stable'
);
PRAGMA foreign_key_check;
```

Если результат не пуст, миграция атомарно останавливается с `publication_migration_policy`. Это относится к beta-only и опубликованным версиям без membership, включая private/archived приложения: их последующее открытие также не должно расширять доступ незаметно. До решения владельца используйте прежний бинарник; статус published и файлы остаются прежними. Автоматического снятия публикации или преобразования данных нет.

Явная политика для конкретного проверенного релиза: владелец разрешает его распространение через общий публичный каталог после перехода. На остановленной старой базе добавьте только одобренный UUID в старый stable membership (current менять не нужно):

```sql
BEGIN IMMEDIATE;
INSERT INTO channel_releases(channel_id,release_id,application_id,added_at)
SELECT c.id,r.id,r.application_id,unixepoch()
FROM releases r JOIN release_channels c ON c.application_id=r.application_id
WHERE r.id='UUID-ПРОВЕРЕННОГО-РЕЛИЗА' AND r.status='published' AND c.name='stable'
ON CONFLICT(channel_id,release_id) DO NOTHING;
COMMIT;
```

Не выполняйте массовое одобрение всего результата без проверки. Если расширение доступа хотя бы к одной версии недопустимо, переход для этой базы следует отложить: новая модель намеренно не вводит скрытые группы или новые статусы взамен каналов. UUID, версия и дата публикации не изменяются даже при явном одобрении.

После одобрения всех обнаруженных версий повторный startup применит миграцию. Проверьте `PRAGMA foreign_key_check`, число и значения релизов/артефактов/подписей, наличие файлов, прямые ссылки и manifest schema_version=2. Политика тестируется на реальной SQLite с foreign_keys=ON: отказ откатывается полностью, явное одобрение позволяет миграцию, подписанные файлы и публикации сохраняются. Исторические миграции 0001–0008 неизменны.

Старые ссылки на точный артефакт без query сохраняют путь. Клиентам старого updater endpoint и манифеста schema_version=1 требуется обновить конфигурацию/парсер: новый endpoint не содержит сегмент канала, манифест использует latest_by_target. Старые query channel у манифеста, каталога и скачиваний отклоняются как неизвестные параметры.
