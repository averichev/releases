import { useEffect, useState } from 'react';
import { Alert, App, Button, Card, Form, Input, Select, Space, Spin, Typography } from 'antd';
import { api } from '../api/client';
import { applicationPath, type Application } from '../api/applications';
import { channelsPath, type Channel, type TauriPreview } from '../api/publication';
import { releaseError, targetName } from '../api/releases';
export default function Manifests({ application }: { application: Application }) {
  const { message } = App.useApp(); const [channels, setChannels] = useState<Channel[]>(); const [channel, setChannel] = useState('stable');
  const [document, setDocument] = useState<string>(); const [error, setError] = useState(''); const [attempt, setAttempt] = useState(0);
  const [preview, setPreview] = useState<TauriPreview>(); const [previewError, setPreviewError] = useState(''); const [busy, setBusy] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    api<Channel[]>(channelsPath(application.id), 'GET', undefined, controller.signal).then((v) => { if (!controller.signal.aborted) setChannels(v); }, (e) => { if (!controller.signal.aborted) setError(releaseError(e)); });
    return () => controller.abort();
  }, [application.id, attempt]);
  useEffect(() => {
    const controller = new AbortController(); setError(''); setDocument(undefined); setPreview(undefined);
    api<unknown>(`${applicationPath(application.id)}/manifests/${encodeURIComponent(channel)}/preview`, 'GET', undefined, controller.signal, 0).then((v) => { if (!controller.signal.aborted) setDocument(JSON.stringify(v, null, 2)); }, (e) => { if (!controller.signal.aborted) setError(releaseError(e)); });
    return () => controller.abort();
  }, [application.id, channel, attempt]);
  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); void message.success('Скопировано'); }
    catch { void message.error('Браузер не разрешил копирование. Выделите текст вручную.'); }
  }
  async function check(values: { current_version: string; target: string }) {
    setBusy(true); setPreview(undefined); setPreviewError('');
    try { setPreview(await api<TauriPreview>(`${applicationPath(application.id)}/updater/tauri/preview`, 'POST', { ...values, channel }, undefined, 0)); }
    catch (e) { setPreviewError(releaseError(e)); } finally { setBusy(false); }
  }
  const url = `${window.location.origin}/${encodeURIComponent(application.slug)}/manifest.json${channel === 'stable' ? '' : `?channel=${encodeURIComponent(channel)}`}`;
  return <Space orientation="vertical" className="full-width">
    <Card title="Манифесты" extra={<Button disabled={busy} onClick={() => setAttempt((v) => v + 1)}>Обновить</Button>}>
      <Space wrap className="release-toolbar"><Select aria-label="Канал манифеста" disabled={busy} value={channel} onChange={setChannel} options={channels?.map((c) => ({ value: c.name, label: c.name }))} /><Button disabled={!document} onClick={() => { if (document) void copy(document); }}>Копировать JSON</Button><Button disabled={application.visibility !== 'public' || application.status !== 'active' || !document} onClick={() => { void copy(url); }}>Копировать публичный URL</Button></Space>
      {application.visibility !== 'public' || application.status !== 'active' ? <Alert className="form-alert" type="info" title="Публичный манифест скрыт. Этот preview доступен только администратору." /> : <Typography.Paragraph copyable>{url}</Typography.Paragraph>}
      {error ? <Alert type="error" title="Манифест недоступен" description={`${error} В пустом канале манифест ещё не сформирован.`} showIcon /> : document ? <pre className="manifest-json" aria-label="JSON манифеста">{document}</pre> : <Spin aria-label="Генерация манифеста" />}
    </Card>
    <Card title="Tauri Updater preview">
      <Alert className="form-alert" type="info" showIcon title="Публичный Tauri Updater v2 подключён." description="Предпросмотр проверяет версию, платформу, подпись и доступность файла. Приватные ключи храните в системе сборки." />
      <Form layout="vertical" disabled={busy} onValuesChange={() => { setPreview(undefined); setPreviewError(''); }} initialValues={{ current_version: '0.0.0', target: 'windows-x86_64' }} onFinish={(values) => { void check(values); }}>
        <Form.Item name="current_version" label="Версия клиента" rules={[{ required: true }]}><Input /></Form.Item>
        <Form.Item name="target" label="Платформа Tauri" rules={[{ required: true }]}><Select options={['windows-x86_64', 'windows-aarch64', 'linux-x86_64', 'linux-aarch64', 'darwin-x86_64', 'darwin-aarch64', 'darwin-universal'].map((target) => ({ value: target, label: targetName(target) }))} /></Form.Item>
        <Button htmlType="submit" loading={busy}>Проверить Tauri</Button>
      </Form>
      {previewError && <Alert className="form-alert" type="error" title={previewError} showIcon />}
      {preview && <><Alert className="form-alert" type={preview.state === 'blocked' ? 'warning' : 'info'} title={preview.message} showIcon /><pre className="manifest-json" aria-label="Диагностика Tauri">{JSON.stringify(preview, null, 2)}</pre></>}
    </Card>
  </Space>;
}
