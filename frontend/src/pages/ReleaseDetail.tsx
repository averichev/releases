import { useEffect, useState } from 'react';
import { Alert, App, Button, Card, Descriptions, Empty, Modal, Result, Space, Spin, Table, Tag, Typography } from 'antd';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { api, ApiError } from '../api/client';
import { applicationPath, date, type Application } from '../api/applications';
import { artifactsPath, releaseError, releasePath, size, targetName, type Artifact, type Release, type ReleaseValues, type UploadOptions } from '../api/releases';
import { useAuth } from '../auth/Auth';
import ReleaseForm from './ReleaseForm';
import ArtifactUploader from './ArtifactUploader';
import ReleasePublication from './ReleasePublication';
import ArtifactUpdater from './ArtifactUpdater';
import DownloadCount from '../components/DownloadCount';
import ReleaseNotes from '../components/ReleaseNotes';
interface Detail { application: Application; release: Release; artifacts: Artifact[]; options: UploadOptions }
export default function ReleaseDetail() {
  const { id = '', releaseId = '' } = useParams(); const { state } = useAuth(); const { modal, message } = App.useApp(); const navigate = useNavigate();
  const administrator = state.kind === 'ready' && state.user.role === 'admin';
  const [detail, setDetail] = useState<Detail>(); const [loadError, setLoadError] = useState(''); const [missing, setMissing] = useState(false); const [attempt, setAttempt] = useState(0);
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const [editing, setEditing] = useState(false); const [uploading, setUploading] = useState(false); const [uploadBusy, setUploadBusy] = useState(false);
  const [refreshBusy, setRefreshBusy] = useState(false);
  useEffect(() => {
    const controller = new AbortController(); setDetail(undefined); setLoadError(''); setMissing(false);
    Promise.all([api<Application>(applicationPath(id), 'GET', undefined, controller.signal), api<Release>(releasePath(id, releaseId), 'GET', undefined, controller.signal), api<Artifact[]>(artifactsPath(id, releaseId), 'GET', undefined, controller.signal), api<UploadOptions>('/admin/artifact-options', 'GET', undefined, controller.signal)]).then(
      ([application, release, artifacts, options]) => { if (!controller.signal.aborted) setDetail({ application, release, artifacts, options }); },
      (error: unknown) => { if (!controller.signal.aborted) { setLoadError(releaseError(error)); setMissing(error instanceof ApiError && error.status === 404); } },
    ); return () => controller.abort();
  }, [id, releaseId, attempt]);
  async function refresh() {
    setRefreshBusy(true);
    try { const [release, artifacts] = await Promise.all([api<Release>(releasePath(id, releaseId)), api<Artifact[]>(artifactsPath(id, releaseId))]); setDetail((current) => current && { ...current, release, artifacts }); }
    catch (error) { setError(releaseError(error)); } finally { setRefreshBusy(false); }
  }
  async function update(values: ReleaseValues) {
    setBusy(true); setError('');
    try { const release = await api<Release>(releasePath(id, releaseId), 'PATCH', values); setDetail((current) => current && { ...current, release }); setEditing(false); void message.success('Релиз сохранён'); }
    catch (error) { setError(releaseError(error)); } finally { setBusy(false); }
  }
  function remove(artifact?: Artifact) {
    modal.confirm({ title: artifact ? 'Удалить артефакт?' : 'Удалить черновик?', content: artifact ? `Файл «${artifact.filename}» будет удалён.` : 'Черновик и все его файлы будут удалены. Восстановить их через панель нельзя.', okText: 'Удалить', cancelText: 'Отмена', okButtonProps: { danger: true },
      onOk: async () => {
        setBusy(true); setError('');
        try { await api(artifact ? `${artifactsPath(id, releaseId)}/${artifact.id}` : releasePath(id, releaseId), 'DELETE');
          if (artifact) { await refresh(); void message.success('Артефакт удалён'); } else navigate(`/admin/applications/${encodeURIComponent(id)}/releases`, { replace: true });
        } catch (error) { setError(releaseError(error)); throw error; } finally { setBusy(false); }
      } });
  }
  async function verify(artifact: Artifact) {
    setBusy(true); setError('');
    try { const result = await api<{ valid: boolean }>(`${artifactsPath(id, releaseId)}/${artifact.id}/verify`, 'POST', undefined, undefined, 0);
      if (result.valid) void message.success(`SHA-256 файла «${artifact.filename}» совпадает`); else setError(`Контрольная сумма файла «${artifact.filename}» не совпадает. Восстановите файл из резервной копии или загрузите его заново.`);
    } catch (error) { setError(releaseError(error)); } finally { setBusy(false); }
  }
  if (loadError) return missing ? <Result status="404" title="Релиз не найден" extra={<Link to={`/admin/applications/${encodeURIComponent(id)}/releases`}>Релизы приложения</Link>} /> : <Alert type="error" showIcon title="Не удалось загрузить релиз" description={loadError} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} />;
  if (!detail) return <div className="catalog-loading" role="status"><Spin /><span>Загружаем релиз…</span></div>;
  const { application, release, artifacts, options } = detail;
  const editable = administrator && release.status === 'draft';
  return <>
    <Link className="back-link" to={`/admin/applications/${encodeURIComponent(id)}/releases`}>← {application.name} / Релизы</Link>
    <div className="release-heading"><div className="page-heading"><Typography.Title level={1}>{release.version}</Typography.Title><Typography.Paragraph>{release.title || application.name}</Typography.Paragraph></div><Tag color={release.status === 'published' ? 'green' : undefined}>{release.status === 'published' ? 'Published' : 'Draft'}</Tag></div>
    {error && <Alert className="form-alert" type="error" title={error} showIcon />}
    <Card className="release-info" title="Информация о релизе" extra={editable && <Button disabled={busy || uploadBusy} onClick={() => { setError(''); setEditing(true); }}>Изменить релиз</Button>}>
      <ReleaseNotes>{release.release_notes || 'Описание изменений не добавлено'}</ReleaseNotes>
      <Descriptions column={{ xs: 1, md: 2 }} size="small" items={[{ key: 'created', label: 'Создан', children: date(release.created_at) }, { key: 'updated', label: 'Изменён', children: date(release.updated_at) }, { key: 'files', label: 'Файлы', children: release.artifact_count }, { key: 'size', label: 'Общий размер', children: size(release.total_size_bytes) }, { key: 'downloads', label: 'Скачивания', children: <DownloadCount count={release.download_count} /> }, ...(release.status === 'published' ? [{ key: 'published', label: 'Опубликован', children: date(release.published_at!) }] : [])]} />
    </Card>
    {editable && <ReleasePublication app={id} release={release} disabled={busy || uploadBusy} onChange={refresh} />}
    <Card className="release-artifacts" title="Артефакты" extra={<Space wrap><Button loading={refreshBusy} onClick={() => { setError(''); void refresh(); }}>Обновить список</Button>{administrator && <Button type="primary" disabled={busy} onClick={() => setUploading(true)}>Добавить артефакт</Button>}</Space>}>
      <Table<Artifact> rowKey="id" dataSource={artifacts} pagination={false} scroll={{ x: 1300 }} locale={{ emptyText: <Empty description="Файлов пока нет" /> }} columns={[
        { title: 'Имя файла', dataIndex: 'filename', width: 160, render: (value: string) => <span className="artifact-filename">{value}</span> }, { title: 'Платформа', dataIndex: 'target', render: targetName }, { title: 'Формат', dataIndex: 'file_type', render: (value: string) => value.toUpperCase() },
        { title: 'Тип артефакта / подпись', render: (_, artifact) => <ArtifactUpdater app={id} release={releaseId} artifact={artifact} editable={editable} disabled={busy || uploadBusy} onChange={refresh} onError={setError} /> },
        { title: 'Размер', dataIndex: 'size_bytes', render: size },
        { title: 'Скачивания', dataIndex: 'download_count', width: 110, align: 'right', render: (count: number) => <DownloadCount count={count} /> },
        { title: 'SHA-256', dataIndex: 'sha256', width: 180, render: (value: string) => <Typography.Text className="artifact-hash" code copyable={{ text: value, tooltips: ['Скопировать SHA-256', 'Скопировано'] }}>{value}</Typography.Text> },
        { title: 'Загружен', dataIndex: 'created_at', render: date }, { title: 'Действия', render: (_, artifact) => <Space orientation="vertical"><a href={`/api/v1${artifactsPath(id, releaseId)}/${artifact.id}/download`}>Скачать</a>{administrator && <><Button type="link" size="small" disabled={busy || uploadBusy} onClick={() => { void verify(artifact); }}>Проверить SHA-256</Button>{editable && <Button type="link" size="small" danger disabled={busy || uploadBusy} onClick={() => remove(artifact)}>Удалить</Button>}</>}</Space> },
      ]} />
    </Card>
    {release.status === 'published' && <PublishedDistribution app={id} release={releaseId} revision={release.revision} />}
    {editable && <Card className="release-delete" title="Удаление черновика"><Typography.Paragraph type="secondary">Удаляется релиз вместе со всеми его файлами.</Typography.Paragraph><Button danger disabled={busy || uploadBusy} onClick={() => remove()}>Удалить черновик</Button></Card>}
    <Modal title="Редактирование релиза" open={editing} footer={null} destroyOnHidden closable={!busy} keyboard={!busy} mask={{ closable: !busy }} onCancel={() => { if (!busy) setEditing(false); }}>{error && <Alert className="form-alert" type="error" title={error} showIcon />}<ReleaseForm release={release} busy={busy} onSubmit={(values) => { void update(values); }} /></Modal>
    <Modal title="Загрузка дистрибутива" width={720} open={uploading} footer={null} destroyOnHidden closable={!uploadBusy} keyboard={!uploadBusy} mask={{ closable: !uploadBusy }} onCancel={() => { if (!uploadBusy) setUploading(false); }}><ArtifactUploader applicationId={id} releaseId={releaseId} artifacts={artifacts} options={options} published={release.status === 'published'} onChange={() => { void refresh(); }} onBusy={setUploadBusy} /></Modal>
  </>;
}

interface ReleaseDistribution { public_available: boolean; files: {id: string; url: string | null; tauri_compatible: boolean; signature_present: boolean}[]; warnings: string[] }
function PublishedDistribution({ app, release, revision }: {app: string; release: string; revision: number}) {
  const [data, setData] = useState<ReleaseDistribution>(); const [error, setError] = useState(''); const [attempt, setAttempt] = useState(0);
  useEffect(() => {const c = new AbortController(); setData(undefined); setError(''); api<ReleaseDistribution>(`${applicationPath(app)}/distribution/releases/${encodeURIComponent(release)}`, 'GET', undefined, c.signal).then((v) => {if (!c.signal.aborted) setData(v);}, (e) => {if (!c.signal.aborted) setError(releaseError(e));}); return () => c.abort();}, [app, release, revision, attempt]);
  return <Card title="Публичное распространение">{error ? <Alert type="error" title={error} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : !data ? <Spin /> : <>
    {!data.public_available && <Alert className="form-alert" type="warning" title="Приложение приватное или архивировано. Публичные ссылки недоступны." />}
    {data.warnings.map((w) => <Alert key={w} className="form-alert" type="warning" title={w} />)}
    <Space orientation="vertical" className="full-width">{data.files.map((f) => <div key={f.id}><Typography.Text code>{f.id}</Typography.Text> <Tag>{f.tauri_compatible ? `Tauri v2 · ${f.signature_present ? 'подпись загружена' : 'нет подписи'}` : 'Обычный дистрибутив'}</Tag>{f.url && <Typography.Paragraph className="distribution-url" copyable={{text: f.url}}><a href={f.url}>{f.url}</a></Typography.Paragraph>}</div>)}</Space>
  </>}</Card>;
}
