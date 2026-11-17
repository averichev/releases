import { useEffect, useState } from 'react';
import { Alert, App, Button, Card, Descriptions, Result, Space, Spin, Tabs, Typography } from 'antd';
import { Link, useLocation, useNavigate, useParams } from 'react-router-dom';
import { useAuth } from '../auth/Auth';
import { api, ApiError } from '../api/client';
import { applicationError, applicationPath, date, saveIcon, type Application, type ApplicationValues } from '../api/applications';
import Releases from './Releases';
import Channels from './Channels';
import Distribution from './Distribution';
import Manifests from './Manifests';
import PublicationHistory from './PublicationHistory';
import ApplicationForm, { ApplicationIcon, ApplicationTags, IconPicker } from './ApplicationForm';

function Settings({ application, onChange, iconError }: { application: Application; onChange: (app: Application) => void; iconError?: string }) {
  const { modal, message } = App.useApp(); const [busy, setBusy] = useState(false); const [error, setError] = useState(iconError ?? ''); const [file, setFile] = useState<File | null>(null);
  async function changeIcon(remove = false) {
    if (!file && !remove) return;
    setBusy(true); setError('');
    try { onChange(remove ? await api<Application>(`${applicationPath(application.id)}/icon`, 'DELETE') : await saveIcon(application.id, file!)); setFile(null); void message.success(remove ? 'Иконка удалена' : 'Иконка обновлена'); }
    catch (error) { setError(applicationError(error)); }
    finally { setBusy(false); }
  }
  async function update(values: ApplicationValues) {
    setBusy(true); setError('');
    try { const { name, description, visibility } = values; onChange(await api<Application>(applicationPath(application.id), 'PATCH', { name, description, visibility })); void message.success('Настройки сохранены'); }
    catch (error) { setError(applicationError(error)); }
    finally { setBusy(false); }
  }
  function confirmStatus() {
    const archived = application.status === 'archived';
    modal.confirm({ title: archived ? 'Восстановить приложение?' : 'Архивировать приложение?', content: 'UUID, slug и публичность сохранятся. Приложение можно восстановить в любой момент.',
      okText: archived ? 'Восстановить' : 'Архивировать', cancelText: 'Отмена', okButtonProps: { danger: !archived },
      onOk: async () => { setBusy(true); setError(''); try { onChange(await api<Application>(`${applicationPath(application.id)}/${archived ? 'restore' : 'archive'}`, 'POST')); void message.success(archived ? 'Приложение восстановлено' : 'Приложение архивировано'); }
        catch (error) { setError(applicationError(error)); throw error; } finally { setBusy(false); } } });
  }
  return <div className="application-settings">
    {error && <Alert className="form-alert" type="error" title={error} showIcon />}
    <Card title="Основные настройки"><ApplicationForm application={application} busy={busy} onSubmit={(values) => { void update(values); }} submitText="Сохранить изменения" /></Card>
    <Card title="Иконка"><Space className="current-icon"><ApplicationIcon application={application} size={64} /><Typography.Text type="secondary">{application.icon_url ? 'Текущая иконка' : 'Стандартная иконка'}</Typography.Text></Space>
      <IconPicker file={file} onChange={setFile} disabled={busy} /><Space wrap><Button type="primary" disabled={!file} loading={busy} onClick={() => { void changeIcon(); }}>Загрузить иконку</Button>
        {application.icon_url && <Button danger disabled={busy} onClick={() => modal.confirm({ title: 'Удалить иконку?', content: 'Будет показана стандартная иконка приложения.', okText: 'Удалить', cancelText: 'Отмена', onOk: () => changeIcon(true) })}>Удалить иконку</Button>}
      </Space>
    </Card>
    <Card title={application.status === 'archived' ? 'Восстановление' : 'Архивация'}><Typography.Paragraph type="secondary">{application.status === 'archived' ? 'Приложение в архиве. Восстановление вернёт его в активный каталог.' : 'Архивное приложение исчезнет из активного каталога. Его идентификаторы и настройки сохранятся.'}</Typography.Paragraph>
      <Button danger={application.status === 'active'} disabled={busy} onClick={confirmStatus}>{application.status === 'archived' ? 'Восстановить приложение' : 'Архивировать приложение'}</Button>
    </Card>
  </div>;
}
export default function ApplicationDetail() {
  const { id = '' } = useParams(); const { state } = useAuth(); const location = useLocation(); const navigate = useNavigate();
  const [application, setApplication] = useState<Application>(); const [error, setError] = useState(''); const [missing, setMissing] = useState(false); const [attempt, setAttempt] = useState(0);
  const administrator = state.kind === 'ready' && state.user.role === 'admin';
  const tab = ['settings', 'releases', 'channels', 'manifests', 'history', 'distribution'].find((key) => location.pathname.endsWith(`/${key}`)) ?? 'overview';
  const base = `/admin/applications/${encodeURIComponent(id)}`;
  useEffect(() => {
    const controller = new AbortController(); setError(''); setMissing(false); setApplication(undefined);
    api<Application>(applicationPath(id), 'GET', undefined, controller.signal).then(
      (app) => { if (!controller.signal.aborted) setApplication(app); },
      (error: unknown) => { if (!controller.signal.aborted) { setError(applicationError(error)); setMissing(error instanceof ApiError && error.status === 404); } },
    ); return () => controller.abort();
  }, [id, attempt]);
  if (error) return missing ? <Result status="404" title="Приложение не найдено" extra={<Link to="/admin/applications">Все приложения</Link>} /> : <Alert type="error" showIcon title="Не удалось загрузить приложение" description={error} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} />;
  if (!application) return <div className="catalog-loading" role="status"><Spin /><span>Загружаем приложение…</span></div>;
  return <>
    <Link className="back-link" to="/admin/applications">← Все приложения</Link>
    <div className="application-detail-heading"><ApplicationIcon application={application} size={72} /><div className="page-heading"><Typography.Title level={1}>{application.name}</Typography.Title><Typography.Text className="application-slug" type="secondary">/{application.slug}</Typography.Text></div></div>
    <ApplicationTags application={application} />
    {application.status === 'archived' && <Alert className="archive-notice" type="warning" showIcon title="Приложение в архиве" description="Публичность сохранена как настройка. Архивный статус сохраняется до восстановления администратором." />}
    <Tabs activeKey={tab} onChange={(key) => navigate(key === 'overview' ? base : `${base}/${key}`)} items={[
      { key: 'overview', label: <Link to={base}>Обзор</Link> }, { key: 'releases', label: <Link to={`${base}/releases`}>Релизы</Link> },
      { key: 'distribution', label: <Link to={`${base}/distribution`}>Распространение</Link> },
      { key: 'channels', label: <Link to={`${base}/channels`}>Каналы обновлений</Link> }, { key: 'history', label: <Link to={`${base}/history`}>История</Link> },
      ...(administrator ? [{ key: 'manifests', label: <Link to={`${base}/manifests`}>Манифесты</Link> }, { key: 'settings', label: <Link to={`${base}/settings`}>Настройки</Link> }] : []),
    ]} />
    {tab === 'overview' && <Card><Typography.Title level={2}>О приложении</Typography.Title><Typography.Paragraph className="application-full-description">{application.description || 'Описание не добавлено'}</Typography.Paragraph>
      <Descriptions column={1} size="small" items={[
        { key: 'id', label: 'UUID', children: <Typography.Text code copyable>{application.id}</Typography.Text> },
        { key: 'slug', label: 'Slug', children: <Typography.Text code>{application.slug}</Typography.Text> },
        { key: 'created', label: 'Создано', children: date(application.created_at) }, { key: 'updated', label: 'Изменено', children: date(application.updated_at) },
      ]} />
    </Card>}
    {tab === 'releases' && <Releases application={application} administrator={administrator} />}
    {tab === 'distribution' && <Distribution application={application} administrator={administrator} />}
    {tab === 'channels' && <Channels application={application} administrator={administrator} />}
    {tab === 'history' && <PublicationHistory app={id} />}
    {tab === 'manifests' && (administrator ? <Manifests application={application} /> : <Result status="403" title="Доступ ограничен" />)}
    {tab === 'settings' && (administrator ? <Settings key={application.id} application={application} onChange={setApplication} iconError={typeof location.state?.iconError === 'string' ? location.state.iconError as string : undefined} /> : <Result status="403" title="Доступ ограничен" subTitle="Изменять настройки приложения может администратор." />)}
  </>;
}
