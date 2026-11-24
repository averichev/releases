import { useEffect, useState } from 'react';
import { Alert, Button, Card, Empty, Table } from 'antd';
import { api } from '../api/client';
import { applicationPath, date } from '../api/applications';
import type { PublicationEvent } from '../api/publication';
import { releaseError } from '../api/releases';
export default function PublicationHistory({ app }: { app: string }) {
  const [events, setEvents] = useState<PublicationEvent[]>([]); const [cursor, setCursor] = useState<number>(); const [more, setMore] = useState(false); const [busy, setBusy] = useState(true); const [error, setError] = useState(''); const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController(); setBusy(true); setError('');
    api<PublicationEvent[]>(`${applicationPath(app)}/publication-history?limit=50${cursor ? `&before=${cursor}` : ''}`, 'GET', undefined, controller.signal).then((rows) => { if (!controller.signal.aborted) { setEvents((previous) => cursor ? [...previous, ...rows] : rows); setMore(rows.length === 50); setBusy(false); } }, (e) => { if (!controller.signal.aborted) { setError(releaseError(e)); setBusy(false); } });
    return () => controller.abort();
  }, [app, cursor, attempt]);
  return <Card title="История публикаций">
    {error && <Alert type="error" title={error} showIcon action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} />}
    <Table<PublicationEvent> rowKey="id" loading={busy} dataSource={events} pagination={false} scroll={{ x: 750 }} locale={{ emptyText: <Empty description="Публикаций пока нет" /> }} columns={[{ title: 'Дата', dataIndex: 'created_at', render: date }, { title: 'Автор', key: 'author', render: (_, event) => event.author ?? event.username }, { title: 'Версия', dataIndex: 'version' }]} />
    {more && <Button disabled={busy} onClick={() => setCursor(events.at(-1)?.id)}>Загрузить ещё</Button>}
  </Card>;
}
