import { useCallback, useEffect, useState } from 'react';
import { App, Alert, Button, Card, Dropdown, Empty, Form, Input, Modal, Radio, Result, Select, Space, Table, Tag, Typography } from 'antd';
import { MoreOutlined, PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import { useAuth } from '../auth/Auth';
import { api, errorMessage, type User, type IssuedLink } from '../api/client';
import { passwordRules } from './Activate';
const roles = { admin: 'Администратор', operator: 'Оператор' };
const statuses = { invited: 'Приглашён', active: 'Активен', disabled: 'Отключён' };
interface CreateValues { username: string; role: User['role']; password?: string }
export default function Users() {
  const auth = useAuth(); const { message, modal } = App.useApp();
  const [users, setUsers] = useState<User[]>([]); const [loading, setLoading] = useState(true); const [error, setError] = useState('');
  const [creating, setCreating] = useState(false); const [mode, setMode] = useState('invitation'); const [busy, setBusy] = useState(false);
  const [link, setLink] = useState<IssuedLink | null>(null); const [form] = Form.useForm<CreateValues>();
  const admin = auth.state.kind === 'ready' && auth.state.user.role === 'admin';
  const load = useCallback(async () => {
    setLoading(true); setError('');
    try { setUsers(await api<User[]>('/admin/users')); } catch (e) { setError(errorMessage(e)); } finally { setLoading(false); }
  }, []);
  useEffect(() => { if (admin) void load(); }, [admin, load]);
  if (!admin) return <Result status="403" title="Доступ ограничен" subTitle="Управление пользователями доступно только администратору." />;
  async function create(values: CreateValues) {
    setBusy(true);
    try {
      if (mode === 'invitation') setLink(await api<IssuedLink>('/admin/users/invitations', 'POST', { username: values.username, role: values.role }));
      else await api('/admin/users', 'POST', values);
      setCreating(false); form.resetFields(); void message.success(mode === 'invitation' ? 'Приглашение создано' : 'Пользователь создан'); await load();
    } catch (e) { void message.error(errorMessage(e)); } finally { setBusy(false); }
  }
  async function action(user: User, key: string) {
    setBusy(true);
    try {
      const path = `/admin/users/${user.id}`;
      if (key === 'invitation' || key === 'password-reset') setLink(await api<IssuedLink>(`${path}/${key}`, 'POST', {}));
      else if (key === 'role') await api(`${path}/role`, 'PATCH', { role: user.role === 'admin' ? 'operator' : 'admin' });
      else if (key === 'status') await api(`${path}/status`, 'PATCH', { status: user.status === 'disabled' ? 'active' : 'disabled' });
      else await api(`${path}/revoke-sessions`, 'POST', {});
      void message.success('Действие выполнено'); await auth.refresh(); await load();
    } catch (e) { void message.error(errorMessage(e)); throw e; } finally { setBusy(false); }
  }
  function confirm(user: User, key: string) {
    const titles: Record<string, string> = { invitation: 'Выдать новое приглашение?', 'password-reset': 'Создать ссылку восстановления пароля?', role: 'Изменить роль пользователя?', status: user.status === 'disabled' ? 'Разблокировать пользователя?' : 'Заблокировать пользователя?', revoke: 'Завершить все сессии пользователя?' };
    const descriptions: Record<string, string> = { invitation: 'Предыдущая ссылка перестанет действовать.', 'password-reset': 'Передайте ссылку лично пользователю. После установки пароля все его сессии завершатся.', role: `Новая роль: ${roles[user.role === 'admin' ? 'operator' : 'admin']}. Все сессии пользователя завершатся.`, status: user.status === 'disabled' ? 'Активированная учётная запись снова сможет войти. Для неактивированной потребуется новое приглашение.' : 'Все сессии завершатся, приглашения и ссылки восстановления будут отозваны.', revoke: 'Для продолжения работы потребуется снова войти.' };
    modal.confirm({ title: titles[key], content: <><Typography.Paragraph><strong>{user.username}</strong></Typography.Paragraph>{descriptions[key]}</>, okText: 'Подтвердить', cancelText: 'Отмена', okButtonProps: { danger: key !== 'invitation' && key !== 'password-reset' }, onOk: () => action(user, key) });
  }
  const activeAdmins = users.filter((u) => u.role === 'admin' && u.status === 'active').length;
  return <><div className="page-heading page-heading-actions"><div><div className="eyebrow">КОМАНДА И ДОСТУП</div><Typography.Title level={1}>Пользователи</Typography.Title><Typography.Paragraph>Учётные записи, роли и доступ к платформе.</Typography.Paragraph></div>
    <Button type="primary" icon={<PlusOutlined />} onClick={() => { setMode('invitation'); setCreating(true); }}>Добавить пользователя</Button></div>
    <Card className="users-card" title="Участники команды" extra={<Button icon={<ReloadOutlined />} aria-label="Обновить список" loading={loading} onClick={() => { void load(); }} />}>
      {error ? <Alert type="error" showIcon title="Не удалось загрузить пользователей" description={error} action={<Button onClick={() => { void load(); }}>Повторить</Button>} /> :
        <Table<User> rowKey="id" dataSource={users} loading={loading} scroll={{ x: 720 }} pagination={{ pageSize: 10, hideOnSinglePage: true }} locale={{ emptyText: <Empty description="Пользователей пока нет" /> }} columns={[
          { title: 'Логин', dataIndex: 'username', render: (value: string, user: User) => <Space><Typography.Text strong>{value}</Typography.Text>{auth.state.kind === 'ready' && user.id === auth.state.user.id && <Tag>Вы</Tag>}</Space> },
          { title: 'Роль', dataIndex: 'role', render: (role: User['role']) => <Tag color={role === 'admin' ? 'green' : 'default'}>{roles[role]}</Tag> },
          { title: 'Статус', dataIndex: 'status', render: (status: User['status']) => <Tag color={status === 'active' ? 'success' : status === 'invited' ? 'processing' : 'default'}>{statuses[status]}</Tag> },
          { title: 'Последний вход', dataIndex: 'last_login_at', render: (value: number | null) => value ? new Date(value * 1000).toLocaleString('ru-RU') : '—' },
          { title: '', key: 'actions', width: 64, render: (_, user) => {
            const last = user.role === 'admin' && user.status === 'active' && activeAdmins === 1;
            return <Dropdown trigger={['click']} menu={{ items: [
              { key: 'role', label: user.role === 'admin' ? 'Сделать оператором' : 'Сделать администратором', disabled: last },
              { key: 'status', label: user.status === 'disabled' ? 'Разблокировать' : 'Заблокировать', danger: user.status !== 'disabled', disabled: last },
              ...(user.status === 'invited' ? [{ key: 'invitation', label: 'Повторить приглашение' }] : []),
              ...(user.status === 'active' ? [{ key: 'password-reset', label: 'Восстановить пароль' }, { key: 'revoke', label: 'Отозвать сессии', danger: true }] : []),
            ], onClick: ({ key }) => confirm(user, key) }}><Button type="text" icon={<MoreOutlined />} aria-label={`Действия: ${user.username}`} disabled={busy} /></Dropdown>;
          } },
        ]} />}
    </Card><Typography.Paragraph className="table-note" type="secondary">Администраторы управляют доступом и приложениями. Операторы могут просматривать каталог и менять собственный пароль в профиле.</Typography.Paragraph>
    <Modal open={creating} title="Новый пользователь" okText={mode === 'invitation' ? 'Создать приглашение' : 'Создать пользователя'} cancelText="Отмена" confirmLoading={busy} onCancel={() => { if (!busy) { setCreating(false); form.resetFields(); } }} onOk={() => form.submit()} destroyOnHidden>
      <Form form={form} layout="vertical" initialValues={{ role: 'operator' }} onFinish={(v) => { void create(v); }} requiredMark={false}>
        <Form.Item label="Способ добавления"><Radio.Group value={mode} onChange={(e) => { setMode(e.target.value as string); form.setFieldValue('password', undefined); }} options={[{ label: 'Приглашение', value: 'invitation' }, { label: 'С паролем', value: 'direct' }]} /></Form.Item>
        <Form.Item name="username" label="Логин" rules={[{ required: true, message: 'Введите логин' }, { pattern: /^[a-zA-Z0-9._-]{3,64}$/, message: 'От 3 до 64 символов: a-z, 0-9, ._-' }]}><Input maxLength={64} autoComplete="off" /></Form.Item>
        <Form.Item name="role" label="Роль" rules={[{ required: true }]}><Select options={[{ value: 'operator', label: roles.operator }, { value: 'admin', label: roles.admin }]} /></Form.Item>
        {mode === 'direct' && <Form.Item name="password" label="Пароль" rules={passwordRules} preserve={false}><Input.Password maxLength={128} autoComplete="new-password" /></Form.Item>}
        <Alert type="info" showIcon title={mode === 'invitation' ? 'Ссылка будет показана один раз' : 'Передайте пароль лично пользователю'} description={mode === 'invitation' ? 'Скопируйте ссылку и передайте её пользователю. Он самостоятельно установит пароль.' : 'Для самостоятельной установки пароля используйте приглашение.'} />
      </Form>
    </Modal>
    <Modal open={link !== null} title="Ссылка для установки пароля" footer={<Button type="primary" onClick={() => setLink(null)}>Ссылка сохранена</Button>} onCancel={() => setLink(null)} destroyOnHidden>
      <Alert className="form-alert" type="warning" showIcon title="Скопируйте ссылку перед закрытием" description="Повторно посмотреть её нельзя. Новая ссылка отменит предыдущую." />
      {link && <><Typography.Paragraph copyable={{ text: link.activation_url }} className="activation-link">{link.activation_url}</Typography.Paragraph><Typography.Text type="secondary">Действует до {new Date(link.expires_at * 1000).toLocaleString('ru-RU')}</Typography.Text></>}
    </Modal>
  </>;
}
