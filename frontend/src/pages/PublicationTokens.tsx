import { useEffect, useState } from 'react';
import { Alert, App, Button, Card, Empty, Form, Input, Modal, Space, Spin, Table, Tag, Typography } from 'antd';
import { api, errorMessage } from '../api/client';
import { applicationPath, date } from '../api/applications';

interface PublicationToken {
  id: string; name: string; created_at: number; expires_at: number | null;
  state: 'active' | 'expired' | 'revoked';
}
const states = { active: 'Действует', expired: 'Истёк', revoked: 'Отозван' };
export default function PublicationTokens({ applicationId, archived }: { applicationId: string; archived: boolean }) {
  const { modal, message } = App.useApp();
  const [tokens, setTokens] = useState<PublicationToken[]>(); const [attempt, setAttempt] = useState(0);
  const [loadError, setLoadError] = useState(''); const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  const [name, setName] = useState(''); const [expiry, setExpiry] = useState(''); const [secret, setSecret] = useState('');
  const path = `${applicationPath(applicationId)}/publication-tokens`;
  useEffect(() => {
    const controller = new AbortController(); setLoadError('');
    api<PublicationToken[]>(path, 'GET', undefined, controller.signal).then(
      (rows) => { if (!controller.signal.aborted) setTokens(rows); },
      (e: unknown) => { if (!controller.signal.aborted) setLoadError(errorMessage(e)); });
    return () => controller.abort();
  }, [path, attempt]);
  async function create() {
    if (!name.trim() || busy) return;
    const expires = expiry ? Math.floor(new Date(expiry).getTime() / 1000) : null;
    if (expires !== null && (!Number.isFinite(expires) || expires <= Date.now() / 1000)) { setError('Срок действия должен быть в будущем.'); return; }
    setBusy(true); setError('');
    try {
      const result = await api<PublicationToken & { token: string }>(path, 'POST', { name: name.trim(), expires_at: expires });
      setSecret(result.token); setName(''); setExpiry(''); setAttempt((v) => v + 1);
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  function revoke(token: PublicationToken) {
    modal.confirm({ title: `Отозвать токен «${token.name}»?`, content: 'Публикация с этим токеном прекратится, включая текущие загрузки. Выпущенные релизы и история сохранятся.', okText: 'Отозвать', cancelText: 'Отмена', okButtonProps: { danger: true },
      onOk: async () => {
        setBusy(true); setError('');
        try { await api(`${path}/${encodeURIComponent(token.id)}/revoke`, 'POST'); setAttempt((v) => v + 1); void message.success('Токен отозван'); }
        catch (e) { setError(errorMessage(e)); throw e; } finally { setBusy(false); }
      } });
  }
  const active = tokens?.filter((t) => t.state === 'active').length ?? 0;
  async function copy() {
    try {
      if (!navigator.clipboard) throw new Error('Clipboard unavailable');
      await navigator.clipboard.writeText(secret); void message.success('Токен скопирован');
    } catch { void message.error('Выделите токен и скопируйте вручную.'); }
  }
  return <Card title="Публикация из CLI">
    <Typography.Paragraph>Один токен публикует новые версии и все платформы этого приложения через curl или CI. Подписи файлов создаются на машине сборки. Токен нужен и для обычного скачивания без подписей.</Typography.Paragraph>
    {loadError ? <Alert type="error" showIcon title="Не удалось загрузить токены" description={loadError} action={<Button onClick={() => setAttempt((v) => v + 1)}>Повторить</Button>} /> : !tokens ? <div role="status"><Spin /> Загружаем токены…</div> : <>
      {error && <Alert className="form-alert" type="error" title={error} showIcon />}
      <Table<PublicationToken> rowKey="id" dataSource={tokens} size="small" pagination={{ pageSize: 5 }} scroll={{ x: 650 }} locale={{ emptyText: <Empty description="Токенов пока нет" /> }} columns={[
        { title: 'Имя', dataIndex: 'name' }, { title: 'Создан', dataIndex: 'created_at', render: date },
        { title: 'Срок действия', dataIndex: 'expires_at', render: (v: number | null) => v === null ? 'Без срока' : date(v) },
        { title: 'Состояние', dataIndex: 'state', render: (v: PublicationToken['state']) => <Tag>{states[v]}</Tag> },
        { title: 'Действие', key: 'action', render: (_, t) => t.state !== 'revoked' && <Button danger size="small" disabled={busy} onClick={() => revoke(t)}>Отозвать</Button> },
      ]} />
      <Typography.Paragraph type="secondary">Одновременно могут действовать два токена. Создайте новый, замените секрет в CI, затем отзовите старый.</Typography.Paragraph>
      {archived && <Alert type="info" title="Для выдачи токена восстановите приложение из архива." showIcon />}
      <Form layout="vertical" disabled={busy || archived || active >= 2} onFinish={() => { void create(); }}>
        <Form.Item label="Имя токена" required><Input aria-label="Имя токена" value={name} maxLength={80} autoComplete="off" onChange={(e) => setName(e.target.value)} placeholder="CI приложения" /></Form.Item>
        <Form.Item label="Действует до" extra="Необязательно. Время в вашем часовом поясе."><Input aria-label="Действует до" type="datetime-local" value={expiry} onChange={(e) => setExpiry(e.target.value)} /></Form.Item>
        <Button htmlType="submit" type="primary" loading={busy} disabled={!name.trim() || archived || active >= 2}>Создать токен</Button>
      </Form>
    </>}
    <Modal title="Сохраните токен" open={!!secret} onCancel={() => setSecret('')} footer={<Button onClick={() => setSecret('')}>Готово</Button>} destroyOnHidden>
      <Alert type="warning" showIcon title="Секрет показан один раз" description="Сохраните его в RELEASES_TOKEN локально или в секретах CI. После закрытия получить его повторно нельзя." />
      <Space orientation="vertical" style={{ width: '100%', marginTop: 16 }}>
        <Input.TextArea aria-label="Токен публикации" value={secret} readOnly autoComplete="off" spellCheck={false} autoSize />
        <Button onClick={() => { void copy(); }}>Скопировать токен</Button>
      </Space>
    </Modal>
  </Card>;
}
