import { useState } from 'react';
import { Alert, App, Button, Card, Descriptions, Modal, Select, Space, Typography } from 'antd';
import { Link } from 'react-router-dom';
import { api } from '../api/client';
import { channelsPath, serverValidation, type Channel, type PublicationCheck, type Validation } from '../api/publication';
import { releaseError, releasePath, type Release } from '../api/releases';
export function ValidationResult({ validation }: { validation: Validation }) {
  return <Space orientation="vertical" className="full-width" role="status">
    <Alert type={validation.state === 'errors' ? 'error' : validation.state === 'warnings' ? 'warning' : 'success'} title={validation.state === 'errors' ? 'Публикация заблокирована' : validation.state === 'warnings' ? 'Готов к публикации с замечаниями' : 'Релиз готов к публикации'} showIcon />
    {validation.errors.map((issue, i) => <Alert key={`e${i}`} type="error" title={issue.message} showIcon />)}
    {validation.warnings.map((issue, i) => <Alert key={`w${i}`} type="warning" title={issue.message} showIcon />)}
  </Space>;
}
export default function ReleasePublication({ app, release, disabled, onChange }: { app: string; release: Release; disabled: boolean; onChange: () => Promise<void> }) {
  const { message } = App.useApp();
  const [channels, setChannels] = useState<Channel[]>([]); const [channel, setChannel] = useState('stable');
  const [check, setCheck] = useState<PublicationCheck>(); const [validation, setValidation] = useState<Validation>();
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const [open, setOpen] = useState(false);
  async function validate() {
    setBusy(true); setError(''); setCheck(undefined); setValidation(undefined);
    try {
      const channels = await api<Channel[]>(channelsPath(app)); setChannels(channels);
      const result = await api<PublicationCheck>(`${releasePath(app, release.id)}/validate-publication`, 'POST', { channel }, undefined, 0);
      await onChange(); setCheck(result); setValidation(result.validation);
    } catch (e) { setError(releaseError(e)); } finally { setBusy(false); }
  }
  async function publish() {
    if (!check) return;
    setBusy(true); setError('');
    try {
      await api(`${releasePath(app, release.id)}/publish`, 'POST', { channel, expected_channel_revision: check.channel.revision, expected_release_revision: check.release_revision }, undefined, 0);
      setOpen(false); setCheck(undefined); setValidation(undefined); await onChange(); void message.success('Релиз опубликован');
    } catch (e) { setError(releaseError(e)); setValidation(serverValidation(e)); setCheck(undefined); } finally { setBusy(false); }
  }
  const validCheck = check && check.release_revision === release.revision && check.channel.name === channel;
  return <Card className="release-info" title="Публикация">
    <Typography.Paragraph>После публикации версия, описание, файлы и подписи будут зафиксированы.</Typography.Paragraph>
    {error && <Alert className="form-alert" type="error" title={error} showIcon />}
    <Space wrap className="release-toolbar">
      <Select id={`publication-channel-${release.id}`} aria-label="Канал публикации" value={channel} disabled={busy} onChange={(value) => { setChannel(value); setCheck(undefined); setValidation(undefined); }} options={(channels.length ? channels : [{ name: 'stable' }, { name: 'beta' }]).map((c) => ({ value: c.name, label: c.name }))} />
      <Button loading={busy} disabled={disabled} onClick={() => { void validate(); }}>Проверить готовность</Button>
      <Button type="primary" disabled={disabled || busy || !validCheck || validation?.state === 'errors'} onClick={() => setOpen(true)}>Опубликовать</Button>
    </Space>
    {validation && (!check || validCheck) && <ValidationResult validation={validation} />}
    <Link to={`/admin/applications/${encodeURIComponent(app)}/channels`}>Каналы обновлений</Link>
    <Modal title="Подтверждение публикации" open={open} okText="Подтвердить публикацию" cancelText="Отмена" confirmLoading={busy} okButtonProps={{ disabled: !validCheck || validation?.state === 'errors' }} closable={!busy} keyboard={!busy} mask={{ closable: !busy }} onCancel={() => { if (!busy) setOpen(false); }} onOk={() => { void publish(); }}>
      <Descriptions column={1} items={[{ key: 'version', label: 'Публикуемая версия', children: release.version }, { key: 'channel', label: 'Канал', children: channel }, { key: 'current', label: 'Текущая версия канала', children: check?.channel.current_version ?? 'Канал пуст' }, { key: 'files', label: 'Количество файлов', children: release.artifact_count }]} />
      {validation && <ValidationResult validation={validation} />}
      {error && <Alert type="error" title={error} showIcon />}
    </Modal>
  </Card>;
}
