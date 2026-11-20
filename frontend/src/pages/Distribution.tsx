import { useEffect, useState } from 'react';
import { Alert, Button, Card, Descriptions, Empty, Space, Spin, Typography } from 'antd';
import { api, errorMessage } from '../api/client';
import { applicationPath, type Application } from '../api/applications';
import { targetName } from '../api/releases';
export interface DistributionSettings {
  public_available: boolean; application_url: string; latest_url: string; manifest_url: string; tauri_url: string;
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
        const doc = await manifest.json() as {latest_by_target: Record<string, {version: string; artifacts: {download_url?: string; id: string}[]}>};
        for (const [target, release] of Object.entries(doc.latest_by_target)) {
          result.push(`${target}: ${release.version}`);
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
  return <Space orientation="vertical" className="full-width">
    {error && <Alert type="error" title={error} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} />}
    {!settings ? !error && <Spin /> : <>
      {!settings.public_available && <Alert type="warning" showIcon title="Публичное распространение недоступно" description="Ссылки начнут работать, когда приложение будет активным и публичным." />}
      <Card title="Ссылки распространения"><Descriptions column={1} items={[
        ['Страница приложения', settings.application_url], ['Выбор установщика', settings.latest_url], ['Манифест', settings.manifest_url], ['Tauri Updater', settings.tauri_url],
      ].map(([label, value]) => ({key: label, label, children: <Typography.Text copyable className="distribution-url">{value}</Typography.Text>}))} /></Card>
      <Card title="Актуальные установщики">{settings.downloads.length ? <Space orientation="vertical" className="full-width">{settings.downloads.map((d) => <div key={d.target}>
        <Typography.Title level={5}>{targetName(d.target)} · v{d.version}</Typography.Title>
        {d.formats.length === 1 ? <Typography.Paragraph copyable className="distribution-url">{d.url}</Typography.Paragraph> : <>
          <Typography.Paragraph type="secondary">Для этой платформы уточните формат в ссылке.</Typography.Paragraph>
          {d.format_urls.map((f) => <Typography.Paragraph key={f.format} copyable className="distribution-url">{f.url}</Typography.Paragraph>)}
        </>}
      </div>)}</Space> : <Empty description="Установщиков пока нет" />}</Card>
      <Card title="Проверка распространения"><Button loading={checking} onClick={() => {void check();}}>Проверить</Button><Typography.Paragraph type="secondary">Проверка манифеста и HEAD актуальных установщиков. Содержимое файлов не скачивается. Диагностика обновлений доступна во вкладке Tauri.</Typography.Paragraph>{diagnostics && <ul>{diagnostics.map((text, i) => <li key={i}>{text}</li>)}</ul>}</Card>
    </>}
  </Space>;
}
