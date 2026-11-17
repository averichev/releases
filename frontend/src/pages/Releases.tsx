import { useEffect, useState } from 'react';
import { Alert, Button, Card, Empty, Input, Modal, Select, Space, Table, Tag } from 'antd';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { api } from '../api/client';
import { date, type Application } from '../api/applications';
import { releaseError, releaseRoute, releasesPath, size, type Release, type ReleasePage, type ReleaseValues } from '../api/releases';
import ReleaseForm from './ReleaseForm';
export default function Releases({ application, administrator }: { application: Application; administrator: boolean }) {
  const navigate = useNavigate(); const [params, setParams] = useSearchParams();
  const [page, setPage] = useState<ReleasePage>(); const [error, setError] = useState(''); const [attempt, setAttempt] = useState(0);
  const [creating, setCreating] = useState(false); const [busy, setBusy] = useState(false); const [createError, setCreateError] = useState('');
  const query = new URLSearchParams({ page: params.get('page') ?? '1', per_page: '20', search: params.get('search') ?? '', sort: params.get('sort') ?? 'version', order: params.get('order') ?? 'desc' }).toString();
  function change(values: Record<string, string>) { setParams({ ...Object.fromEntries(params), page: '1', ...values }); }
  useEffect(() => {
    const controller = new AbortController(); setError(''); setPage(undefined);
    api<ReleasePage>(`${releasesPath(application.id)}?${query}`, 'GET', undefined, controller.signal).then((value) => { if (!controller.signal.aborted) setPage(value); }, (error: unknown) => { if (!controller.signal.aborted) setError(releaseError(error)); });
    return () => controller.abort();
  }, [application.id, query, attempt]);
  async function create(values: ReleaseValues) {
    setBusy(true); setCreateError('');
    try { const release = await api<Release>(releasesPath(application.id), 'POST', values); navigate(releaseRoute(application.id, release.id)); }
    catch (error) { setCreateError(releaseError(error)); } finally { setBusy(false); }
  }
  return <Card title="Релизы" extra={administrator && <Button type="primary" disabled={application.status === 'archived'} onClick={() => { setCreateError(''); setCreating(true); }}>Создать релиз</Button>}>
    <Space wrap className="release-toolbar">
      <Input.Search key={params.get('search') ?? ''} aria-label="Поиск версий" placeholder="Поиск по версии" defaultValue={params.get('search') ?? ''} allowClear onSearch={(search) => change({ search })} />
      <Select aria-label="Сортировка релизов" value={params.get('sort') ?? 'version'} onChange={(sort: string) => change({ sort })} options={[{ value: 'version', label: 'По версии (SemVer)' }, { value: 'created_at', label: 'По дате создания' }, { value: 'updated_at', label: 'По дате изменения' }]} />
      <Select aria-label="Порядок релизов" value={params.get('order') ?? 'desc'} onChange={(order: string) => change({ order })} options={[{ value: 'desc', label: 'По убыванию' }, { value: 'asc', label: 'По возрастанию' }]} />
    </Space>
    {error ? <Alert type="error" showIcon title="Не удалось загрузить релизы" description={error} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : <Table<Release> rowKey="id" loading={!page} dataSource={page?.items} scroll={{ x: 850 }} locale={{ emptyText: <Empty description="Релизов пока нет" /> }} pagination={{ current: page?.page ?? 1, total: page?.total ?? 0, pageSize: 20, showSizeChanger: false, onChange: (page) => change({ page: String(page) }) }} columns={[
      { title: 'Версия', dataIndex: 'version', render: (version: string, release) => <Link to={releaseRoute(application.id, release.id)}>{version}</Link> },
      { title: 'Название', dataIndex: 'title', render: (value: string | null) => value || '—' },
      { title: 'Статус', render: (_, release) => <Tag color={release.status === 'published' ? 'green' : undefined}>{release.status === 'published' ? 'Published' : 'Draft'}</Tag> }, { title: 'Создан', dataIndex: 'created_at', render: date },
      { title: 'Файлы', dataIndex: 'artifact_count' }, { title: 'Размер', dataIndex: 'total_size_bytes', render: size },
      { title: 'Действия', render: (_, release) => <Link to={releaseRoute(application.id, release.id)}>Открыть</Link> },
    ]} />}
    <Modal title="Новый релиз" open={creating} footer={null} destroyOnHidden closable={!busy} keyboard={!busy} mask={{ closable: !busy }} onCancel={() => { if (!busy) setCreating(false); }}>
      {createError && <Alert className="form-alert" type="error" title={createError} showIcon />}<ReleaseForm busy={busy} onSubmit={(values) => { void create(values); }} />
    </Modal>
  </Card>;
}
