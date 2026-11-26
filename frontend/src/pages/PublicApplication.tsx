import { useEffect, useState } from 'react';
import { Alert, App, Avatar, Button, Card, Empty, List, Pagination, Result, Select, Space, Spin, Tag, Tooltip, Typography } from 'antd';
import { CopyOutlined, DownloadOutlined, AppstoreOutlined } from '@ant-design/icons';
import { useParams } from 'react-router-dom';
import { api, ApiError, errorMessage } from '../api/client';
import { artifactTypeName, size, targetName, targetOs } from '../api/releases';
import { PublicLayout } from '../components/PublicLayout';
import ReleaseNotes from '../components/ReleaseNotes';
export interface PublicArtifact { id: string; filename: string; target: string; format: string; purpose: string; size_bytes: number; download_url: string }
export interface PublicRelease { version: string; published_at: string; notes: string; artifacts: PublicArtifact[] }
interface PublicApp { name: string; description: string; slug: string; icon_url: string | null; manifest_url: string; latest_by_target: Record<string, PublicRelease>; latest_by_type?: Record<string, Record<string, PublicRelease>> }
interface History { items: PublicRelease[]; total: number }
const releaseDate = (s: string) => new Date(s).toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: 'numeric' });
function Files({ release }: { release: PublicRelease }) {
  return <List dataSource={release.artifacts} renderItem={(a) => <List.Item><Space wrap><a href={a.download_url}>{a.filename}</a><Tag>{targetName(a.target)}</Tag><Tag>{artifactTypeName(a.purpose)}</Tag><Typography.Text type="secondary">{a.format.toUpperCase()} · {size(a.size_bytes)}</Typography.Text></Space></List.Item>} />;
}
function Download({ app }: { app: PublicApp }) {
  const [selectedOs, setSelectedOs] = useState('all');
  const [selectedKind, setSelectedKind] = useState('all');
  const downloads = Object.entries(app.latest_by_type ?? {installer: app.latest_by_target})
    .flatMap(([kind, targets]) => Object.entries(targets).map(([target, release]) => ({kind, target, os: targetOs(target), release})));
  if (!downloads.length) return <Empty description="Файлов для скачивания пока нет" />;
  const systems = [...new Set(downloads.map(({os}) => os))].filter((os) => os !== 'any');
  const kinds = [...new Set(downloads.map(({kind}) => kind))];
  const visible = downloads.filter(({os, kind}) =>
    (selectedOs === 'all' || selectedOs === os || os === 'any') && (selectedKind === 'all' || selectedKind === kind));
  return <section aria-label="Актуальные файлы"><Typography.Title level={2}>Файлы для скачивания</Typography.Title>
    {(systems.length > 1 || kinds.length > 1) && <div className="public-download-filters">
      {systems.length > 1 && <div>
        <label htmlFor="download-os">Операционная система</label>
        <Select id="download-os" value={selectedOs} onChange={setSelectedOs}
          options={[{value: 'all', label: 'Все ОС'}, ...systems.map((os) => ({value: os, label: targetName(os)}))]} />
      </div>}
      {kinds.length > 1 && <div>
        <label htmlFor="download-kind">Тип файла</label>
        <Select id="download-kind" value={selectedKind} onChange={setSelectedKind}
          options={[{value: 'all', label: 'Все типы'}, ...kinds.map((kind) => ({value: kind, label: artifactTypeName(kind)}))]} />
      </div>}
    </div>}
    {!visible.length && <Empty description="Для выбранной ОС и типа файлов пока нет" />}
    <Space orientation="vertical" className="full-width">
      {visible.map(({kind, target, release}) => <Card key={`${kind}-${target}`} title={`${targetName(target)} · ${artifactTypeName(kind)}`} extra={<Tag>v{release.version}</Tag>}>
        <Typography.Paragraph type="secondary">{releaseDate(release.published_at)}</Typography.Paragraph>
        <Space orientation="vertical" className="full-width">{release.artifacts.map((a) => <Space key={a.id} wrap>
          <Button type="primary" icon={<DownloadOutlined />} href={a.download_url} aria-label={`Скачать ${a.format.toUpperCase()} · ${targetName(target)}`}>Скачать {a.format.toUpperCase()}</Button>
          <Typography.Text>{a.filename} · {size(a.size_bytes)}</Typography.Text>
          <Typography.Text copyable={{text: a.download_url}}>Ссылка</Typography.Text>
        </Space>)}</Space>
        {release.notes && <ReleaseNotes>{release.notes}</ReleaseNotes>}
      </Card>)}
    </Space></section>;
}
export default function PublicApplication() {
  const { slug = '' } = useParams();
  const { message } = App.useApp();
  const [app, setApp] = useState<PublicApp>(); const [history, setHistory] = useState<History>(); const [page, setPage] = useState(1);
  const [error, setError] = useState(''); const [historyError, setHistoryError] = useState(''); const [missing, setMissing] = useState(false); const [attempt, setAttempt] = useState(0);
  const base = `/public/applications/${encodeURIComponent(slug)}`;
  async function copyManifest(url: string) {
    try { await navigator.clipboard.writeText(url); void message.success('Ссылка на манифест скопирована'); }
    catch { void message.error('Не удалось скопировать. Скопируйте адрес через меню ссылки.'); }
  }
  useEffect(() => {
    const c = new AbortController(); setApp(undefined); setError(''); setMissing(false); setPage(1);
    api<PublicApp>(base, 'GET', undefined, c.signal).then((v) => { if (!c.signal.aborted) setApp(v); }, (e) => { if (!c.signal.aborted) { setError(errorMessage(e)); setMissing(e instanceof ApiError && e.status === 404); } });
    return () => c.abort();
  }, [base, attempt]);
  useEffect(() => {
    const c = new AbortController(); setHistory(undefined); setHistoryError('');
    api<History>(`${base}/releases?limit=10&offset=${(page - 1) * 10}`, 'GET', undefined, c.signal).then((v) => { if (!c.signal.aborted) setHistory(v); }, (e) => { if (!c.signal.aborted) setHistoryError(errorMessage(e)); });
    return () => c.abort();
  }, [base, page, attempt]);
  return <PublicLayout>
    {error ? <Result status={missing ? '404' : 'error'} title={missing ? 'Приложение не найдено' : 'Не удалось загрузить приложение'} subTitle={missing ? 'Проверьте адрес приложения.' : error} extra={!missing && <Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : !app ? <div className="catalog-loading"><Spin aria-label="Загрузка приложения" /></div> : <>
      <section className="public-hero"><Avatar shape="square" size={88} src={app.icon_url} icon={<AppstoreOutlined />} /><div><Typography.Text className="eyebrow">РАСПРОСТРАНЕНИЕ ПРИЛОЖЕНИЙ</Typography.Text><Typography.Title level={1}>{app.name}</Typography.Title><Typography.Paragraph className="application-full-description">{app.description}</Typography.Paragraph>
        <Space size={4}><Typography.Link href={app.manifest_url}>Манифест JSON</Typography.Link><Tooltip title="Скопировать ссылку на манифест"><Button type="text" size="small" icon={<CopyOutlined />} aria-label="Скопировать ссылку на манифест" onClick={() => { void copyManifest(app.manifest_url); }} /></Tooltip></Space>
      </div></section>
      <Download app={app} />
      <section><Typography.Title level={2}>История версий</Typography.Title>{historyError ? <Alert type="error" title="Не удалось загрузить историю" description={historyError} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : !history ? <Spin aria-label="Загрузка истории" /> : <><Space orientation="vertical" className="full-width">{history.items.length ? history.items.map((r) => <Card key={r.version} title={`Версия ${r.version}`} extra={<Typography.Text type="secondary">{releaseDate(r.published_at)}</Typography.Text>}><ReleaseNotes>{r.notes || 'Описание изменений не добавлено'}</ReleaseNotes><Files release={r} /></Card>) : <Empty description="Опубликованных версий пока нет" />}</Space>{history.total > 10 && <Pagination current={page} pageSize={10} total={history.total} onChange={setPage} showSizeChanger={false} />}</>}</section>
    </>}
  </PublicLayout>;
}
