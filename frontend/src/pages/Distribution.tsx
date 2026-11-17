import { useEffect, useState } from 'react';
import { Alert, Button, Card, Descriptions, Form, Input, Select, Space, Spin, Typography } from 'antd';
import { api, errorMessage } from '../api/client';
import { applicationPath, type Application } from '../api/applications';
import { targetName, type UploadOptions } from '../api/releases';
interface Selector { target: string; format: string; purpose: 'installer' }
export interface DistributionSettings { public_available: boolean; application_url: string; latest_url: string; stable_manifest_url: string; beta_manifest_url: string; tauri_url: string; default_distribution: Selector | null; channels: {name: string; version: string | null}[] }
export default function Distribution({ application, administrator }: { application: Application; administrator: boolean }) {
  const [settings, setSettings] = useState<DistributionSettings>(); const [options, setOptions] = useState<UploadOptions>(); const [error, setError] = useState('');
  const [busy, setBusy] = useState(false); const [attempt, setAttempt] = useState(0); const [diagnostics, setDiagnostics] = useState<string[]>(); const [checking, setChecking] = useState(false); const [channel, setChannel] = useState('stable');
  const base = `${applicationPath(application.id)}/distribution`;
  useEffect(() => { const c = new AbortController(); setError(''); setSettings(undefined);
    Promise.all([api<DistributionSettings>(base, 'GET', undefined, c.signal), api<UploadOptions>('/admin/artifact-options', 'GET', undefined, c.signal)]).then(([s, o]) => { if (!c.signal.aborted) {setSettings(s); setOptions(o);} }, (e) => {if (!c.signal.aborted) setError(errorMessage(e));}); return () => c.abort();
  }, [base, attempt]);
  async function save(value: Selector | null) { setBusy(true); setError(''); try {await api(`${base}/default`, 'PUT', value);setAttempt((v) => v + 1);} catch (e) {setError(errorMessage(e));} finally {setBusy(false);} }
  async function check() {
    setChecking(true); setDiagnostics(undefined); const result: string[] = [];
    const root = `/${encodeURIComponent(application.slug)}`;
    const request = (path: string, method: string) => fetch(path, {method, credentials: 'omit', cache: 'no-store', signal: AbortSignal.timeout(15000)});
    try {
      const manifest = await request(`${root}/manifest.json?channel=${channel}`, 'GET');
      result.push(`Манифест ${channel}: HTTP ${manifest.status}`);
      if (manifest.ok) {
        const doc = await manifest.json() as { latest: string; releases: {artifacts: {download_url?: string; id: string}[]}[] };
        const expected = settings?.channels.find((c) => c.name === channel)?.version;
        result.push(`Текущий релиз: ${doc.latest}${doc.latest === expected ? ' — соответствует каналу' : ' — канал изменился, обновите данные'}`);
        for (const a of doc.releases[0]?.artifacts ?? []) {
          const url = a.download_url ? new URL(a.download_url) : null;
          const valid = url && url.origin === new URL(settings!.application_url).origin && url.pathname.startsWith(`${root}/releases/`) && url.pathname.endsWith(`/artifacts/${encodeURIComponent(a.id)}/download`) && (!url.search || url.search === '?channel=beta');
          if (!valid || !url) {result.push(`Артефакт ${a.id}: некорректный URL`);continue;}
          const head = await request(`${url.pathname}${url.search}`, 'HEAD');
          result.push(`Артефакт ${a.id}: HEAD ${head.status}, размер ${head.headers.get('content-length') ?? 'не указан'}`);
        }
      }
      if (administrator) {
        const preview = await api<{message: string}>(`${applicationPath(application.id)}/updater/tauri/preview`, 'POST', {channel, target: settings?.default_distribution?.target === 'any' ? 'windows-x86_64' : settings?.default_distribution?.target ?? 'windows-x86_64', current_version: '0.0.0'});
        result.push(`Tauri: ${preview.message} Проверка остальных платформ доступна во вкладке «Манифесты».`);
      }
    } catch (e) {result.push(errorMessage(e));} finally {setDiagnostics(result);setChecking(false);}
  }
  return <Space orientation="vertical" className="full-width">
    {error && <Alert type="error" title={error} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} />}
    {!settings ? !error && <Spin /> : <>
      {!settings.public_available && <Alert type="warning" showIcon title="Публичное распространение недоступно" description="Ссылки начнут работать, когда приложение будет активным и публичным." />}
      <Card title="Ссылки распространения"><Descriptions column={1} items={[
        ['Страница приложения', settings.application_url], ['Последняя stable', settings.latest_url], ['Манифест stable', settings.stable_manifest_url], ['Манифест beta', settings.beta_manifest_url], ['Tauri Updater', settings.tauri_url],
      ].map(([label, value]) => ({key: label, label, children: <Typography.Text copyable className="distribution-url">{value}</Typography.Text>}))} /><Space wrap>{settings.channels.map((c) => <Typography.Text key={c.name}>{c.name}: {c.version ?? 'нет релиза'}</Typography.Text>)}</Space></Card>
      <Card title="Дистрибутив по умолчанию"><Typography.Paragraph type="secondary">Единственный установщик выбирается автоматически. При нескольких файлах используется этот селектор. Пакеты с назначением updater выбираются только явно.</Typography.Paragraph>
        {administrator ? <Form key={attempt} layout="vertical" initialValues={settings.default_distribution ?? {purpose: 'installer'}} onFinish={(v: Selector) => {void save({...v, purpose: 'installer'});}} disabled={busy}>
          <Form.Item name="target" label="Платформа" rules={[{required: true}]}><Select options={options?.targets.map((t) => ({value: t, label: targetName(t)}))} /></Form.Item>
          <Form.Item name="format" label="Формат" rules={[{required: true}, {pattern: /^[a-z0-9][a-z0-9._-]{0,31}$/}]}><Input placeholder="exe, msi, appimage, dmg…" /></Form.Item>
          <Space wrap><Button type="primary" htmlType="submit" loading={busy}>Сохранить</Button><Button disabled={busy || !settings.default_distribution} onClick={() => {void save(null);}}>Сбросить</Button></Space>
        </Form> : <Typography.Text>{settings.default_distribution ? `${targetName(settings.default_distribution.target)} / ${settings.default_distribution.format}` : 'Не задан'}</Typography.Text>}
      </Card>
      <Card title="Проверка распространения"><Space wrap><Select aria-label="Канал проверки" value={channel} onChange={setChannel} options={[{value:'stable'}, {value:'beta'}]} /><Button loading={checking} onClick={() => {void check();}}>Проверить</Button></Space><Typography.Paragraph type="secondary">Проверка манифеста и HEAD файлов текущего сервиса. Содержимое дистрибутивов не скачивается.</Typography.Paragraph>{diagnostics && <ListResults items={diagnostics} />}</Card>
    </>}
  </Space>;
}
function ListResults({ items }: {items: string[]}) { return <ul>{items.map((text, i) => <li key={i}>{text}</li>)}</ul>; }
