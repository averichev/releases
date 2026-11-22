import { useEffect, useState } from 'react';
import { Alert, Button, Card, Descriptions, Empty, Space, Spin, Typography } from 'antd';
import { api, errorMessage } from '../api/client';
import { applicationPath, type Application } from '../api/applications';
import { artifactTypeName, targetName } from '../api/releases';
export interface DistributionSettings {
  public_available: boolean; application_url: string; latest_url: string; manifest_url: string; tauri_url: string;
  artifact_downloads?: {type: string; target: string; version: string; files: {format: string; url: string}[]}[];
  downloads: {target: string; version: string; formats: string[]; url: string; format_urls: {format: string; url: string}[]}[];
}
export default function Distribution({ application }: { application: Application }) {
  const [settings, setSettings] = useState<DistributionSettings>(); const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0); const [diagnostics, setDiagnostics] = useState<string[]>(); const [checking, setChecking] = useState(false);
  const base = `${applicationPath(application.id)}/distribution`;
  useEffect(() => {
    const c = new AbortController(); setError(''); setSettings(undefined); setDiagnostics(undefined);
    api<DistributionSettings>(base, 'GET', undefined, c.signal).then((s) => { if (!c.signal.aborted) setSettings(s); }, (e) => {if (!c.signal.aborted) setError(errorMessage(e));});
    return () => c.abort();
  }, [base, attempt]);
  async function check() {
    setChecking(true); setDiagnostics(undefined); const result: string[] = [];
    const root = `/${encodeURIComponent(application.slug)}`;
    const request = (path: string, method: string) => fetch(path, {method, credentials: 'omit', cache: 'no-store', signal: AbortSignal.timeout(15000)});
    try {
      const manifest = await request(`${root}/manifest.json`, 'GET');
      result.push(`Манифест: HTTP ${manifest.status}`);
      if (manifest.ok) {
        type Release = {version: string; artifacts: {download_url?: string; id: string}[]};
        const doc = await manifest.json() as {latest_by_target: Record<string, Release>; latest_by_type?: Record<string, Record<string, Release>>};
        const downloads = Object.entries(doc.latest_by_type ?? {installer: doc.latest_by_target}).flatMap(([kind, targets]) => Object.entries(targets).map(([target, release]) => ({kind, target, release})));
        for (const {kind, target, release} of downloads) {
          result.push(`${artifactTypeName(kind)} · ${target}: ${release.version}`);
          for (const a of release.artifacts) {
            const url = a.download_url ? new URL(a.download_url) : null;
            const valid = url && url.origin === new URL(settings!.application_url).origin && url.pathname.startsWith(`${root}/releases/`) && url.pathname.endsWith(`/artifacts/${encodeURIComponent(a.id)}/download`) && !url.search && !url.hash;
            if (!valid || !url) {result.push(`Артефакт ${a.id}: некорректный URL`);continue;}
            const head = await request(url.pathname, 'HEAD');
            result.push(`Артефакт ${a.id}: HEAD ${head.status}, размер ${head.headers.get('content-length') ?? 'не указан'}`);
          }
        }
      }
    } catch (e) {result.push(errorMessage(e));} finally {setDiagnostics(result);setChecking(false);}
  }
  const downloads = settings?.artifact_downloads ?? settings?.downloads.map((d) => ({type: 'installer', target: d.target, version: d.version, files: d.format_urls})) ?? [];
  return <Space orientation="vertical" className="full-width">
    {error && <Alert type="error" title={error} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} />}
    {!settings ? !error && <Spin /> : <>
      {!settings.public_available && <Alert type="warning" showIcon title="Публичное распространение недоступно" description="Ссылки начнут работать, когда приложение будет активным и публичным." />}
      <Card title="Ссылки распространения"><Descriptions column={1} items={[
        ['Страница приложения', settings.application_url], ['Выбор файла', settings.latest_url], ['Манифест', settings.manifest_url], ['Tauri Updater', settings.tauri_url],
      ].map(([label, value]) => ({key: label, label, children: <Typography.Text copyable className="distribution-url">{value}</Typography.Text>}))} /></Card>
      <Card title="Актуальные файлы">{downloads.length ? <Space orientation="vertical" className="full-width">{downloads.map((d) => <div key={`${d.type}-${d.target}`}>
        <Typography.Title level={5}>{targetName(d.target)} · {artifactTypeName(d.type)} · v{d.version}</Typography.Title>
        {d.files.map((f) => <Typography.Paragraph key={f.format} copyable className="distribution-url">{f.url}</Typography.Paragraph>)}
      </div>)}</Space> : <Empty description="Файлов для скачивания пока нет" />}</Card>
      <Card title="Проверка распространения"><Button loading={checking} onClick={() => {void check();}}>Проверить</Button><Typography.Paragraph type="secondary">Проверка манифеста и HEAD актуальных файлов. Содержимое файлов не скачивается. Диагностика обновлений доступна во вкладке Tauri.</Typography.Paragraph>{diagnostics && <ul>{diagnostics.map((text, i) => <li key={i}>{text}</li>)}</ul>}</Card>
    </>}
  </Space>;
}
