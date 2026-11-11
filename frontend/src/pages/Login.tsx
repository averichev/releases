import { useState } from 'react';
import { Alert, Button, Card, Form, Input, Typography, App } from 'antd';
import { LockOutlined, UserOutlined } from '@ant-design/icons';
import { useLocation, useNavigate, Link } from 'react-router-dom';
import { useAuth, SessionLoading } from '../auth/Auth';
import { errorMessage } from '../api/client';
export default function Login() {
  const auth = useAuth(); const navigate = useNavigate(); const location = useLocation(); const { message } = App.useApp();
  const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  async function submit(values: { username: string; password: string }) {
    setBusy(true); setError('');
    try {
      await auth.login(values.username, values.password);
      const from: unknown = location.state?.from;
      navigate(typeof from === 'string' && from.startsWith('/admin/') && !from.startsWith('/admin/login') && !from.startsWith('/admin/activate') ? from : '/admin', { replace: true });
    } catch (error) { setError(errorMessage(error)); } finally { setBusy(false); }
  }
  if (auth.state.kind === 'loading') return <SessionLoading />;
  return <main className="auth-page"><div className="auth-container">
    <div className="auth-brand"><span className="brand-mark">v</span><div>Veruna <span>RELEASES</span></div></div>
    <Card className="auth-card"><div className="eyebrow">ПАНЕЛЬ УПРАВЛЕНИЯ</div><Typography.Title level={1}>Вход в систему</Typography.Title>
      <Typography.Paragraph type="secondary">Управляйте доступом команды к платформе.</Typography.Paragraph>
      {auth.state.kind === 'ready' ? <><Alert type="success" title={`Вы вошли как ${auth.state.user.username}`} />
        <div className="auth-actions"><Link to="/admin"><Button type="primary">Открыть панель</Button></Link>
          <Button loading={busy} onClick={async () => { setBusy(true); try { await auth.logout(); } catch (e) { void message.error(errorMessage(e)); } finally { setBusy(false); } }}>Сменить пользователя</Button></div></> :
        <Form layout="vertical" onFinish={(v: { username: string; password: string }) => { void submit(v); }} requiredMark={false}>
          {error && <Alert className="form-alert" type="error" showIcon title={error} role="alert" />}
          <Form.Item label="Логин" name="username" rules={[{ required: true, message: 'Введите логин' }]}><Input autoComplete="username" maxLength={64} prefix={<UserOutlined />} autoFocus /></Form.Item>
          <Form.Item label="Пароль" name="password" rules={[{ required: true, message: 'Введите пароль' }]}><Input.Password autoComplete="current-password" maxLength={128} prefix={<LockOutlined />} /></Form.Item>
          <Button block type="primary" htmlType="submit" size="large" loading={busy}>Войти</Button>
        </Form>}
    </Card><Typography.Paragraph className="auth-note">Первого администратора создаёт владелец сервера через CLI.<br />Для получения доступа обратитесь к администратору.</Typography.Paragraph>
  </div></main>;
}
