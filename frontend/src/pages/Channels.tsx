import { useEffect, useState } from 'react';
import { Alert, App, Button, Card, Descriptions, Empty, Input, Modal, Select, Space, Table, Tag, Typography } from 'antd';
import { api } from '../api/client';
import { date, type Application } from '../api/applications';
import { channelsPath, serverValidation, type Channel, type Validation } from '../api/publication';
import { releaseError, releasesPath, type Release, type ReleasePage } from '../api/releases';
import { ValidationResult } from './ReleasePublication';
export default function Channels({ application, administrator }: { application: Application; administrator: boolean }) {
  const { message } = App.useApp(); const [channels, setChannels] = useState<Channel[]>(); const [error, setError] = useState(''); const [attempt, setAttempt] = useState(0);
  const [channel, setChannel] = useState<Channel>(); const [candidates, setCandidates] = useState<ReleasePage>(); const [selected, setSelected] = useState<Release>();
  const [page, setPage] = useState(1); const [search, setSearch] = useState(''); const [busy, setBusy] = useState(false); const [candidateError, setCandidateError] = useState(''); const [validation, setValidation] = useState<Validation>();
  useEffect(() => {
    const controller = new AbortController(); setError(''); setChannels(undefined);
    api<Channel[]>(channelsPath(application.id), 'GET', undefined, controller.signal).then((value) => { if (!controller.signal.aborted) setChannels(value); }, (e) => { if (!controller.signal.aborted) setError(releaseError(e)); });
    return () => controller.abort();
  }, [application.id, attempt]);
  useEffect(() => {
    if (!channel) return;
    const controller = new AbortController(); setCandidates(undefined); setCandidateError('');
    api<ReleasePage>(`${releasesPath(application.id)}?${new URLSearchParams({ status: 'published', page: String(page), per_page: '20', search })}`, 'GET', undefined, controller.signal).then((value) => { if (!controller.signal.aborted) setCandidates(value); }, (e) => { if (!controller.signal.aborted) setCandidateError(releaseError(e)); });
    return () => controller.abort();
  }, [application.id, channel, page, search]);
  async function switchRelease() {
    if (!channel || !selected) return;
    setBusy(true); setCandidateError(''); setValidation(undefined);
    try {
      await api(`${channelsPath(application.id)}/${encodeURIComponent(channel.name)}/current`, 'PUT', { release_id: selected.id, expected_channel_revision: channel.revision, expected_release_revision: selected.revision }, undefined, 0);
      setChannel(undefined); setAttempt((v) => v + 1); void message.success('Текущий релиз канала изменён');
    } catch (e) { setCandidateError(releaseError(e)); setValidation(serverValidation(e)); } finally { setBusy(false); }
  }
  return <Card title="Каналы обновлений" extra={<Button disabled={busy} onClick={() => setAttempt((v) => v + 1)}>Обновить каналы</Button>}>
    {error ? <Alert type="error" title={error} showIcon action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : <Table<Channel> rowKey="id" dataSource={channels} loading={!channels} pagination={false} scroll={{ x: 750 }} columns={[
      { title: 'Канал', dataIndex: 'name' }, { title: 'Текущая версия', dataIndex: 'current_version', render: (v: string | null) => v ?? '—' }, { title: 'Дата публикации', dataIndex: 'published_at', render: (v: number | null) => v ? date(v) : '—' },
      { title: 'Состояние', render: (_, c) => <Tag color={c.current_release_id ? 'green' : undefined}>{c.current_release_id ? 'Опубликован' : 'Пустой канал'}</Tag> }, { title: 'Релизы', dataIndex: 'release_count' },
      ...(administrator ? [{ title: 'Действия', render: (_: unknown, c: Channel) => <Button disabled={application.status !== 'active'} onClick={() => { setChannel(c); setSelected(undefined); setValidation(undefined); setPage(1); setSearch(''); }}>Переключить версию</Button> }] : []),
    ]} />}
    <Modal title={`Переключение канала ${channel?.name ?? ''}`} open={!!channel} onCancel={() => { if (!busy) setChannel(undefined); }} onOk={() => { void switchRelease(); }} okText="Подтвердить переключение" cancelText="Отмена" confirmLoading={busy} okButtonProps={{ disabled: !selected || selected.id === channel?.current_release_id }} closable={!busy} keyboard={!busy} mask={{ closable: !busy }}>
      <Alert className="form-alert" type="warning" showIcon title="Откат канала не гарантирует downgrade уже установленных приложений." description="Канал изменяет версию, предлагаемую сервером. Автоматическая установка старой версии не выполняется." />
      <Descriptions column={1} items={[{ key: 'from', label: 'Текущая версия', children: channel?.current_version ?? 'Канал пуст' }, { key: 'to', label: 'Новая версия', children: selected?.version ?? 'Не выбрана' }]} />
      <Space orientation="vertical" className="full-width">
        <Input.Search disabled={busy} aria-label="Поиск опубликованной версии" placeholder="Поиск опубликованной версии" onSearch={(v) => { setSearch(v); setPage(1); }} />
        <Select className="full-width" aria-label="Новая версия канала" disabled={busy} placeholder="Выберите опубликованный релиз" value={selected?.id} loading={!candidates} options={candidates?.items.map((r) => ({ value: r.id, label: `${r.version}${r.id === channel?.current_release_id ? ' · текущая' : ''}` }))} onChange={(id) => { setSelected(candidates?.items.find((r) => r.id === id)); setValidation(undefined); }} notFoundContent={<Empty description="Опубликованных версий нет" />} />
        {candidates && candidates.total > 20 && <Space><Button disabled={busy || page === 1} onClick={() => setPage((p) => p - 1)}>Предыдущие</Button><Typography.Text>{page}</Typography.Text><Button disabled={busy || page * 20 >= candidates.total} onClick={() => setPage((p) => p + 1)}>Следующие</Button></Space>}
        {candidateError && <Alert type="error" title={candidateError} showIcon action={<Button disabled={busy} onClick={() => { setChannel(undefined); setAttempt((v) => v + 1); }}>Обновить данные</Button>} />}{validation && <ValidationResult validation={validation} />}
      </Space>
    </Modal>
  </Card>;
}
