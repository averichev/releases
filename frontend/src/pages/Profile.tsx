import { useState } from 'react';
import { App, Alert, Button, Card, Descriptions, Form, Input, Typography } from 'antd';
import { useAuth } from '../auth/Auth';
import { api, errorMessage } from '../api/client';
import { passwordRules, ConfirmPassword } from './Activate';
export default function Profile() {
  const auth = useAuth(); const { message, modal } = App.useApp(); const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  if (auth.state.kind !== 'ready') return null;
  const user = auth.state.user;
  async function change(values: { current_password: string; new_password: string }) {
    setBusy(true); setError('');
    try { await api('/auth/change-password', 'POST', { current_password: values.current_password, new_password: values.new_password });
      void message.success('Пароль изменён. Все сессии завершены. Войдите снова.'); await auth.refresh();
    } catch (error) { setError(errorMessage(error)); } finally { setBusy(false); }
  }
  return <><div className="page-heading"><div className="eyebrow">УЧЁТНАЯ ЗАПИСЬ</div><Typography.Title level={1}>Профиль</Typography.Title><Typography.Paragraph>Личные данные и безопасность доступа.</Typography.Paragraph></div>
    <div className="profile-grid"><Card title="Текущий пользователь"><Descriptions column={1} items={[
      { key: 'username', label: 'Логин', children: user.username }, { key: 'role', label: 'Роль', children: user.role === 'admin' ? 'Администратор' : 'Оператор' },
      { key: 'login', label: 'Последний вход', children: user.last_login_at ? new Date(user.last_login_at * 1000).toLocaleString('ru-RU') : '—' },
    ]} /><Button className="logout-button" onClick={() => { modal.confirm({ title: 'Выйти из системы?', okText: 'Выйти', cancelText: 'Отмена', onOk: async () => { try { await auth.logout(); } catch (e) { void message.error(errorMessage(e)); throw e; } } }); }}>Выйти из системы</Button></Card>
      <Card title="Смена пароля"><Form layout="vertical" requiredMark={false} onFinish={(v: { current_password: string; new_password: string }) => { void change(v); }}>
        {error && <Alert type="error" className="form-alert" title={error} role="alert" />}
        <Form.Item name="current_password" label="Текущий пароль" rules={[{ required: true, message: 'Введите текущий пароль' }]}><Input.Password autoComplete="current-password" maxLength={128} /></Form.Item>
        <Form.Item name="new_password" label="Новый пароль" rules={passwordRules}><Input.Password autoComplete="new-password" maxLength={128} /></Form.Item><ConfirmPassword field="new_password" />
        <Typography.Paragraph type="secondary">После смены пароля потребуется войти снова на всех устройствах.</Typography.Paragraph><Button type="primary" htmlType="submit" loading={busy}>Изменить пароль</Button>
      </Form></Card></div></>;
}
