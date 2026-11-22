import { useEffect, useState } from 'react';
import { Alert, Avatar, Button, Card, Empty, List, Pagination, Result, Space, Spin, Tag, Typography } from 'antd';
import { DownloadOutlined, AppstoreOutlined } from '@ant-design/icons';
import { useParams } from 'react-router-dom';
import { api, ApiError, errorMessage } from '../api/client';
import { artifactTypeName, size, targetName } from '../api/releases';
import { PublicLayout } from '../components/PublicLayout';
export interface PublicArtifact { id: string; filename: string; target: string; format: string; purpose: string; size_bytes: number; download_url: string }
export interface PublicRelease { version: string; published_at: string; notes: string; artifacts: PublicArtifact[] }
interface PublicApp { name: string; description: string; slug: string; icon_url: string | null; latest_by_target: Record<string, PublicRelease>; latest_by_type?: Record<string, Record<string, PublicRelease>> }
interface History { items: PublicRelease[]; total: number }
const releaseDate = (s: string) => new Date(s).toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: 'numeric' });
function Files({ release }: { release: PublicRelease }) {
  return <List dataSource={release.artifacts} renderItem={(a) => <List.Item><Space wrap><a href={a.download_url}>{a.filename}</a><Tag>{targetName(a.target)}</Tag><Tag>{artifactTypeName(a.purpose)}</Tag><Typography.Text type="secondary">{a.format.toUpperCase()} · {size(a.size_bytes)}</Typography.Text></Space></List.Item>} />;
}
function Download({ app }: { app: PublicApp }) {
  const downloads = Object.entries(app.latest_by_type ?? {installer: app.latest_by_target}).flatMap(([kind, targets]) => Object.entries(targets).map(([target, release]) => ({kind, target, release})));
  if (!downloads.length) return <Empty description="Файлов для скачивания пока нет" />;
  return <section aria-label="Актуальные файлы"><Typography.Title level={2}>Скачать приложение</Typography.Title><Space orientation="vertical" className="full-width">
    {downloads.map(({kind, target, release}) => <Card key={`${kind}-${target}`} title={`${targetName(target)} · ${artifactTypeName(kind)}`} extra={<Tag>v{release.version}</Tag>}>
      <Typography.Paragraph type="secondary">{releaseDate(release.published_at)}</Typography.Paragraph>
      <Space orientation="vertical" className="full-width">{release.artifacts.map((a) => <Space key={a.id} wrap>
        <Button type="primary" icon={<DownloadOutlined />} href={a.download_url}>Скачать {a.format.toUpperCase()}</Button>
        <Typography.Text>{a.filename} · {size(a.size_bytes)}</Typography.Text>
        <Typography.Text copyable={{text: a.download_url}}>Ссылка</Typography.Text>
      </Space>)}</Space>
      {release.notes && <Typography.Paragraph className="application-full-description">{release.notes}</Typography.Paragraph>}
    </Card>)}
  </Space></section>;
}
export default function PublicApplication() {
  const { slug = '' } = useParams();
  const [app, setApp] = useState<PublicApp>(); const [history, setHistory] = useState<History>(); const [page, setPage] = useState(1);
  const [error, setError] = useState(''); const [historyError, setHistoryError] = useState(''); const [missing, setMissing] = useState(false); const [attempt, setAttempt] = useState(0);
  const base = `/public/applications/${encodeURIComponent(slug)}`;
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
      <section className="public-hero"><Avatar shape="square" size={88} src={app.icon_url} icon={<AppstoreOutlined />} /><div><Typography.Text className="eyebrow">РАСПРОСТРАНЕНИЕ ПРИЛОЖЕНИЙ</Typography.Text><Typography.Title level={1}>{app.name}</Typography.Title><Typography.Paragraph className="application-full-description">{app.description}</Typography.Paragraph></div></section>
      <Download app={app} />
      <section><Typography.Title level={2}>История версий</Typography.Title>{historyError ? <Alert type="error" title="Не удалось загрузить историю" description={historyError} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : !history ? <Spin aria-label="Загрузка истории" /> : <><Space orientation="vertical" className="full-width">{history.items.length ? history.items.map((r) => <Card key={r.version} title={`Версия ${r.version}`} extra={<Typography.Text type="secondary">{releaseDate(r.published_at)}</Typography.Text>}><Typography.Paragraph className="application-full-description">{r.notes || 'Описание изменений не добавлено'}</Typography.Paragraph><Files release={r} /></Card>) : <Empty description="Опубликованных версий пока нет" />}</Space>{history.total > 10 && <Pagination current={page} pageSize={10} total={history.total} onChange={setPage} showSizeChanger={false} />}</>}</section>
    </>}
  </PublicLayout>;
}
