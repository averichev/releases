import { useEffect, useState } from 'react';
import { Alert, Avatar, Button, Card, Empty, Layout, List, Pagination, Result, Select, Space, Spin, Tag, Typography } from 'antd';
import { DownloadOutlined, AppstoreOutlined } from '@ant-design/icons';
import { useParams, useSearchParams } from 'react-router-dom';
import { api, ApiError, errorMessage } from '../api/client';
import { size, targetName } from '../api/releases';
import { Brand } from '../components/Brand';
import { ServiceVersion } from '../components/ServiceVersion';
export interface PublicArtifact { id: string; filename: string; target: string; format: string; purpose: string; size_bytes: number; download_url: string }
export interface PublicRelease { version: string; published_at: string; notes: string; artifacts: PublicArtifact[] }
interface PublicApp { name: string; description: string; slug: string; icon_url: string | null; channel: string; latest: PublicRelease | null; default_artifact_id: string | null }
interface History { items: PublicRelease[]; total: number }
const releaseDate = (s: string) => new Date(s).toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: 'numeric' });
const osName = (s: string) => ({ windows: 'Windows', linux: 'Linux', darwin: 'macOS', any: 'Любая ОС' })[s] ?? s;
const parts = (a: PublicArtifact): [string, string] => { const [os = 'any', arch = 'any'] = a.target.split('-'); return [os, arch]; };
function Files({ release }: { release: PublicRelease }) {
  return <List dataSource={release.artifacts} renderItem={(a) => <List.Item><Space wrap><a href={a.download_url}>{a.filename}</a><Tag>{targetName(a.target)}</Tag><Typography.Text type="secondary">{size(a.size_bytes)} · {a.purpose === 'updater' ? 'Пакет обновления' : 'Установщик'}</Typography.Text></Space></List.Item>} />;
}
function Download({ app }: { app: PublicApp }) {
  const files = app.latest?.artifacts.filter((a) => a.purpose === 'installer') ?? [];
  const initial = files.find((a) => a.id === app.default_artifact_id);
  const [os, setOs] = useState(initial ? parts(initial)[0] : undefined);
  const [arch, setArch] = useState(initial ? parts(initial)[1] ?? 'any' : undefined);
  const [format, setFormat] = useState(initial?.format);
  const candidates = files.filter((a) => (!os || parts(a)[0] === os) && (!arch || (parts(a)[1] ?? 'any') === arch) && (!format || a.format === format));
  const selected = candidates.length === 1 ? candidates[0] : undefined;
  const options = (values: string[], label?: (s: string) => string) => [...new Set(values)].map((value) => ({ value, label: label?.(value) ?? value }));
  if (!app.latest) return <Empty description={app.channel === 'beta' ? 'Релизы beta ещё не опубликованы' : 'Стабильные релизы ещё не опубликованы'} />;
  if (!files.length) return <Empty description="В этом релизе нет установщиков для ручного скачивания" />;
  return <Card className="public-download" title="Скачать приложение">
    <div className="public-selectors">
      <label>Операционная система<Select aria-label="Операционная система" placeholder="Выберите ОС" allowClear value={os} onChange={(v) => { setOs(v); setArch(undefined); setFormat(undefined); }} options={options(files.map((a) => parts(a)[0]), osName)} /></label>
      <label>Архитектура<Select aria-label="Архитектура" placeholder="Выберите архитектуру" allowClear value={arch} onChange={(v) => { setArch(v); setFormat(undefined); }} options={options(files.filter((a) => !os || parts(a)[0] === os).map((a) => parts(a)[1] ?? 'any'))} /></label>
      <label>Формат<Select aria-label="Формат" placeholder="Выберите формат" allowClear value={format} onChange={setFormat} options={options(files.filter((a) => (!os || parts(a)[0] === os) && (!arch || (parts(a)[1] ?? 'any') === arch)).map((a) => a.format), (s) => s.toUpperCase())} /></label>
    </div>
    <Space orientation="vertical" className="full-width">
      <Button type="primary" size="large" icon={<DownloadOutlined />} href={selected?.download_url} disabled={!selected}>Скачать {app.name}</Button>
      {selected ? <Typography.Text type="secondary">{targetName(selected.target)} · {selected.format.toUpperCase()} · {size(selected.size_bytes)} · v{app.latest.version}</Typography.Text> : <Typography.Text type="secondary">{candidates.length ? 'Выберите платформу и формат установщика.' : 'Для выбранных параметров файл отсутствует.'}</Typography.Text>}
    </Space>
  </Card>;
}
export default function PublicApplication() {
  const { slug = '' } = useParams(); const [query] = useSearchParams(); const channel = query.get('channel') === 'beta' ? 'beta' : 'stable';
  const [app, setApp] = useState<PublicApp>(); const [history, setHistory] = useState<History>(); const [page, setPage] = useState(1);
  const [error, setError] = useState(''); const [historyError, setHistoryError] = useState(''); const [missing, setMissing] = useState(false); const [attempt, setAttempt] = useState(0);
  const base = `/public/applications/${encodeURIComponent(slug)}`;
  useEffect(() => {
    const c = new AbortController(); setApp(undefined); setError(''); setMissing(false); setPage(1);
    api<PublicApp>(`${base}?channel=${channel}`, 'GET', undefined, c.signal).then((v) => { if (!c.signal.aborted) setApp(v); }, (e) => { if (!c.signal.aborted) { setError(errorMessage(e)); setMissing(e instanceof ApiError && e.status === 404); } });
    return () => c.abort();
  }, [base, channel, attempt]);
  useEffect(() => {
    const c = new AbortController(); setHistory(undefined); setHistoryError('');
    api<History>(`${base}/releases?channel=${channel}&limit=10&offset=${(page - 1) * 10}`, 'GET', undefined, c.signal).then((v) => { if (!c.signal.aborted) setHistory(v); }, (e) => { if (!c.signal.aborted) setHistoryError(errorMessage(e)); });
    return () => c.abort();
  }, [base, channel, page, attempt]);
  return <Layout className="public-layout"><Layout.Header className="public-header"><Brand /></Layout.Header><Layout.Content className="public-content">
    {error ? <Result status={missing ? '404' : 'error'} title={missing ? 'Приложение не найдено' : 'Не удалось загрузить приложение'} subTitle={missing ? 'Проверьте адрес приложения.' : error} extra={!missing && <Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : !app ? <div className="catalog-loading"><Spin aria-label="Загрузка приложения" /></div> : <>
      <section className="public-hero"><Avatar shape="square" size={88} src={app.icon_url} icon={<AppstoreOutlined />} /><div><Typography.Text className="eyebrow">РАСПРОСТРАНЕНИЕ ПРИЛОЖЕНИЙ</Typography.Text><Typography.Title level={1}>{app.name}</Typography.Title><Typography.Paragraph className="application-full-description">{app.description}</Typography.Paragraph><Space wrap><Tag color={channel === 'beta' ? 'orange' : 'green'}>{channel}</Tag>{app.latest && <><Tag>v{app.latest.version}</Tag><Typography.Text type="secondary">{releaseDate(app.latest.published_at)}</Typography.Text></>}</Space></div></section>
      <Download key={`${app.slug}-${app.latest?.version}-${app.default_artifact_id}`} app={app} />
      {app.latest && <Card title="Что нового"><Typography.Paragraph className="application-full-description">{app.latest.notes || 'Описание изменений не добавлено'}</Typography.Paragraph><Space wrap>{[...new Set(app.latest.artifacts.map((a) => a.target))].map((t) => <Tag key={t}>{targetName(t)}</Tag>)}</Space></Card>}
      <section><Typography.Title level={2}>История версий</Typography.Title>{historyError ? <Alert type="error" title="Не удалось загрузить историю" description={historyError} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : !history ? <Spin aria-label="Загрузка истории" /> : <><Space orientation="vertical" className="full-width">{history.items.length ? history.items.map((r) => <Card key={r.version} title={`Версия ${r.version}`} extra={<Typography.Text type="secondary">{releaseDate(r.published_at)}</Typography.Text>}><Typography.Paragraph className="application-full-description">{r.notes || 'Описание изменений не добавлено'}</Typography.Paragraph><Files release={r} /></Card>) : <Empty description="Опубликованных версий пока нет" />}</Space>{history.total > 10 && <Pagination current={page} pageSize={10} total={history.total} onChange={setPage} showSizeChanger={false} />}</>}</section>
    </>}
  </Layout.Content><Layout.Footer className="public-footer"><Brand /><span>Распространение приложений</span><ServiceVersion /></Layout.Footer></Layout>;
}
