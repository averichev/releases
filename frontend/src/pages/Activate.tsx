import { useEffect, useState } from 'react';
import { Alert, Button, Card, Form, Input, Result, Spin, Typography } from 'antd';
import { Link, useLocation, useNavigate } from 'react-router-dom';
import { api, errorMessage } from '../api/client';
export const passwordRules = [{ required: true, message: 'Введите пароль' }, { min: 12, max: 128, message: 'От 12 до 128 символов' }];
export function ConfirmPassword({ field = 'password' }: { field?: string }) {
  return <Form.Item label="Подтвердите пароль" name="confirmation" dependencies={[field]} rules={[
    { required: true, message: 'Повторите пароль' },
    ({ getFieldValue }) => ({ validator: (_, value: string) => !value || getFieldValue(field) === value ? Promise.resolve() : Promise.reject(new Error('Пароли не совпадают')) }),
  ]}><Input.Password autoComplete="new-password" maxLength={128} /></Form.Item>;
}
type TokenState = { kind: 'loading' } | { kind: 'ready'; username: string; purpose: string } | { kind: 'error'; message: string } | { kind: 'done' };
export default function Activate() {
  const location = useLocation(); const navigate = useNavigate();
  const token = new URLSearchParams(location.hash.slice(1)).get('token') ?? '';
  const [state, setState] = useState<TokenState>({ kind: 'loading' }); const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    api<{ username: string; purpose: string }>('/auth/token/check', 'POST', { token }, controller.signal).then(
      (data) => { if (!controller.signal.aborted) setState({ kind: 'ready', ...data }); },
      (error: unknown) => { if (!controller.signal.aborted) setState({ kind: 'error', message: errorMessage(error) }); },
    ); return () => controller.abort();
  }, [token]);
  async function submit(values: { password: string }) {
    setBusy(true); setError('');
    try { await api('/auth/activate', 'POST', { token, password: values.password }); setState({ kind: 'done' });
      navigate('/admin/activate', { replace: true, state: { done: true } });
    } catch (error) { setError(errorMessage(error)); } finally { setBusy(false); }
  }
  const done = state.kind === 'done' || location.state?.done === true;
  return <main className="auth-page"><Card className="auth-container auth-card">
    {done ? <Result status="success" title="Пароль установлен" subTitle="Теперь можно войти в свою учётную запись." extra={<Link to="/admin/login"><Button type="primary">Перейти ко входу</Button></Link>} /> :
      state.kind === 'loading' ? <Spin aria-label="Проверяем ссылку" /> : state.kind === 'error' ?
        <Result status="warning" title="Не удалось открыть ссылку" subTitle={state.message} extra={<Link to="/admin/login">Вернуться ко входу</Link>} /> : state.kind === 'ready' ? <>
          <div className="eyebrow">VERUNA RELEASES</div><Typography.Title level={1}>{state.purpose === 'reset' ? 'Новый пароль' : 'Активация доступа'}</Typography.Title>
          <Typography.Paragraph>Учётная запись: <strong>{state.username}</strong></Typography.Paragraph>
          <Form layout="vertical" onFinish={(values: { password: string }) => { void submit(values); }} requiredMark={false}>
            {error && <Alert className="form-alert" type="error" title={error} role="alert" />}
            <Form.Item label="Пароль" name="password" rules={passwordRules} extra="От 12 до 128 символов. Используйте уникальную парольную фразу."><Input.Password autoComplete="new-password" maxLength={128} /></Form.Item>
            <ConfirmPassword /><Button type="primary" block htmlType="submit" loading={busy}>Установить пароль</Button>
          </Form></> : null}
  </Card></main>;
}
