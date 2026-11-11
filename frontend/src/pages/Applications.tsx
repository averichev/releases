import { useEffect, useState } from 'react';
import { Alert, App, Button, Card, Dropdown, Empty, Input, Pagination, Segmented, Select, Space, Spin, Table, Typography } from 'antd';
import { AppstoreOutlined, MoreOutlined, PlusOutlined, ReloadOutlined, UnorderedListOutlined } from '@ant-design/icons';
import { Link, useSearchParams } from 'react-router-dom';
import { useAuth } from '../auth/Auth';
import { api } from '../api/client';
import { applicationError, applicationPath, date, type Application, type ApplicationPage } from '../api/applications';
import { ApplicationIcon, ApplicationTags } from './ApplicationForm';

export default function Applications() {
  const { state } = useAuth(); const { modal, message } = App.useApp();
  const administrator = state.kind === 'ready' && state.user.role === 'admin';
  const [params, setParams] = useSearchParams(); const query = params.toString();
  const [data, setData] = useState<ApplicationPage>(); const [error, setError] = useState(''); const [loading, setLoading] = useState(true);
  const [attempt, setAttempt] = useState(0); const [view, setView] = useState<string>('cards'); const [search, setSearch] = useState(params.get('search') ?? '');
  useEffect(() => { setSearch(params.get('search') ?? ''); }, [params]);
  useEffect(() => {
    const controller = new AbortController(); setLoading(true); setError('');
    api<ApplicationPage>(`/admin/applications?${query}`, 'GET', undefined, controller.signal).then(
      (value) => { if (!controller.signal.aborted) { setData(value); setLoading(false); } },
      (error: unknown) => { if (!controller.signal.aborted) { setError(applicationError(error)); setLoading(false); } },
    );
    return () => controller.abort();
  }, [query, attempt]);
  function filter(values: Record<string, string>) { const next = new URLSearchParams(params); next.set('page', '1'); for (const [key, value] of Object.entries(values)) { if (value) next.set(key, value); else next.delete(key); } setParams(next); }
  function confirmStatus(app: Application) {
    const archived = app.status === 'archived';
    modal.confirm({ title: archived ? `Восстановить «${app.name}»?` : `Архивировать «${app.name}»?`,
      content: archived ? 'Приложение вернётся в активный каталог. Публичность сохранится.' : 'Приложение останется в базе. Его можно будет найти в архиве и восстановить.',
      okText: archived ? 'Восстановить' : 'Архивировать', cancelText: 'Отмена', okButtonProps: { danger: !archived },
      onOk: async () => {
        try { await api(`${applicationPath(app.id)}/${archived ? 'restore' : 'archive'}`, 'POST'); void message.success(archived ? 'Приложение восстановлено' : 'Приложение архивировано');
          if (data?.items.length === 1 && data.page > 1) { const next = new URLSearchParams(params); next.set('page', String(data.page - 1)); setParams(next); } else setAttempt((v) => v + 1);
        } catch (error) { void message.error(applicationError(error)); throw error; }
      } });
  }
  function actions(app: Application) {
    return <Dropdown menu={{ items: [
      { key: 'open', label: <Link to={`/admin/applications/${app.id}`}>Открыть</Link> },
      ...(administrator ? [
        { key: 'settings', label: <Link to={`/admin/applications/${app.id}/settings`}>Настройки</Link> },
        { key: 'status', label: app.status === 'archived' ? 'Восстановить' : 'Архивировать', danger: app.status === 'active', onClick: () => confirmStatus(app) },
      ] : []),
    ] }}><Button type="text" aria-label={`Действия: ${app.name}`} icon={<MoreOutlined />} /></Dropdown>;
  }
  return <>
    <div className="page-heading page-heading-actions"><div><div className="eyebrow">КАТАЛОГ ПРОДУКТОВ</div><Typography.Title level={1}>Приложения</Typography.Title><Typography.Paragraph>Продукты, их постоянные адреса и настройки распространения.</Typography.Paragraph></div>
      {administrator && <Link to="/admin/applications/new"><Button type="primary" icon={<PlusOutlined />}>Создать приложение</Button></Link>}
    </div>
    <Card className="catalog-filters"><div className="catalog-toolbar">
      <Input.Search aria-label="Поиск приложений" placeholder="Название или slug" value={search} maxLength={120} allowClear onChange={(e) => { setSearch(e.target.value); if (!e.target.value) filter({ search: '' }); }} onSearch={(value) => filter({ search: value.trim() })} />
      <Select aria-label="Статус приложения" value={params.get('status') ?? 'active'} onChange={(status) => filter({ status })} options={[{ value: 'active', label: 'Активные' }, { value: 'archived', label: 'Архивные' }, { value: 'all', label: 'Все статусы' }]} />
      <Select aria-label="Публичность приложения" value={params.get('visibility') ?? 'all'} onChange={(visibility) => filter({ visibility })} options={[{ value: 'all', label: 'Любая публичность' }, { value: 'private', label: 'Приватные' }, { value: 'public', label: 'Публичные' }]} />
      <Select aria-label="Сортировка приложений" value={`${params.get('sort') ?? 'updated_at'}:${params.get('order') ?? 'desc'}`} onChange={(value) => { const [sort = 'updated_at', order = 'desc'] = value.split(':'); filter({ sort, order }); }} options={[
        { value: 'updated_at:desc', label: 'Недавно изменённые' }, { value: 'updated_at:asc', label: 'Давно изменённые' },
        { value: 'created_at:desc', label: 'Сначала новые' }, { value: 'created_at:asc', label: 'Сначала старые' },
        { value: 'name:asc', label: 'Название: А → Я' }, { value: 'name:desc', label: 'Название: Я → А' },
      ]} />
    </div></Card>
    <div className="catalog-results"><Typography.Text type="secondary">{!loading && !error && data ? `Найдено: ${data.total}` : 'Каталог приложений'}</Typography.Text><Space>
      <Button aria-label="Обновить каталог" icon={<ReloadOutlined />} loading={loading} onClick={() => setAttempt((v) => v + 1)} />
      <Segmented aria-label="Вид каталога" value={view} onChange={setView} options={[{ value: 'cards', icon: <AppstoreOutlined />, title: 'Карточки' }, { value: 'table', icon: <UnorderedListOutlined />, title: 'Таблица' }]} />
    </Space></div>
    {loading ? <div className="catalog-loading" role="status"><Spin /><span>Загружаем приложения…</span></div> : error ? <Alert type="error" showIcon title="Не удалось загрузить каталог" description={error} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : data?.items.length ? <>
      {view === 'cards' ? <div className="application-grid">{data.items.map((app) => <Card key={app.id} className="application-card">
        <div className="application-card-header"><ApplicationIcon application={app} />{actions(app)}</div>
        <Typography.Title level={2}><Link to={`/admin/applications/${app.id}`}>{app.name}</Link></Typography.Title>
        <Typography.Text className="application-slug" type="secondary">/{app.slug}</Typography.Text>
        <Typography.Paragraph className="application-description" ellipsis={{ rows: 3 }}>{app.description || 'Описание не добавлено'}</Typography.Paragraph>
        <ApplicationTags application={app} /><Typography.Text className="application-updated" type="secondary">Изменено {date(app.updated_at)}</Typography.Text>
      </Card>)}</div> : <Card className="catalog-table"><Table rowKey="id" dataSource={data.items} pagination={false} scroll={{ x: 850 }} columns={[
        { title: 'Приложение', dataIndex: 'name', render: (_, app) => <Space><ApplicationIcon application={app} size={36} /><div><Link to={`/admin/applications/${app.id}`}>{app.name}</Link><div className="application-slug">/{app.slug}</div></div></Space> },
        { title: 'Описание', dataIndex: 'description', ellipsis: true, render: (v: string) => v || '—' },
        { title: 'Состояние', key: 'status', render: (_, app) => <ApplicationTags application={app} /> },
        { title: 'Изменено', dataIndex: 'updated_at', render: date },
        { title: '', key: 'actions', width: 52, render: (_, app) => actions(app) },
      ]} /></Card>}
      <Pagination className="catalog-pagination" current={data.page} pageSize={data.per_page} total={data.total} showSizeChanger pageSizeOptions={[12, 24, 48, 96]} onChange={(page, size) => { const next = new URLSearchParams(params); next.set('page', String(size === data.per_page ? page : 1)); next.set('per_page', String(size)); setParams(next); }} />
    </> : <Card><Empty description={params.get('search') || params.get('visibility') && params.get('visibility') !== 'all' ? 'По заданным условиям приложения не найдены' : params.get('status') === 'archived' ? 'В архиве пока нет приложений' : 'Приложений пока нет'}>
      {administrator && <Link to="/admin/applications/new"><Button type="primary">Создать приложение</Button></Link>}
    </Empty></Card>}
  </>;
}
