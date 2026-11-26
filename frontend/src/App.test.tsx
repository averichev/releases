import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { App } from 'antd';
import { expect, it, vi } from 'vitest';
import { AppRoutes } from './App';
const user = { id: 'id', username: 'owner', role: 'admin', status: 'active', created_at: 1, updated_at: 1, last_login_at: 1 };
const session = { user, csrf_token: 'csrf-value' };
const response = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status });
function renderAt(path: string) { return render(<App><MemoryRouter initialEntries={[path]}><AppRoutes /></MemoryRouter></App>); }
const unauthorized = () => response({ error: { code: 'unauthorized' } }, 401);
it('denies the service update page to operators without polling the admin endpoint', async () => {
  const fetch = vi.fn(() => Promise.resolve(response({ ...session, user: { ...user, role: 'operator' } })));
  vi.stubGlobal('fetch', fetch);
  renderAt('/admin/self-update');
  await screen.findByText('Требуются права администратора');
  expect(screen.queryByRole('link', { name: 'Обновление сервиса' })).toBeNull();
  expect(fetch).not.toHaveBeenCalledWith('/api/v1/admin/self-update', expect.anything());
});

it('requires confirmation for the same prepared update before restarting the service', async () => {
  const status = { state: 'prepared', operation_id: 'prepared-id', available_version: '1.1.0' };
  const summary = { mode: 'supervisor', current_version: '1.0.0', target: 'linux-x86_64', installation_supported: true, status };
  const fetch = vi.fn((url: string, _init?: RequestInit) => Promise.resolve(response(
    url.endsWith('/auth/me') ? session : url.endsWith('/install') ? status : summary,
  )));
  vi.stubGlobal('fetch', fetch);
  renderAt('/admin/self-update');
  fireEvent.click(await screen.findByRole('button', { name: 'Установить и перезапустить' }));
  const dialog = await screen.findByRole('dialog', { name: 'Установить версию 1.1.0?' });
  expect(fetch.mock.calls.some(([url]) => url.endsWith('/install'))).toBe(false);
  fireEvent.click(within(dialog).getByRole('button', { name: 'Установить' }));
  await waitFor(() => expect(fetch).toHaveBeenCalledWith('/api/v1/admin/self-update/install',
    expect.objectContaining({ method: 'POST', body: JSON.stringify({ operation_id: 'prepared-id' }),
      headers: expect.objectContaining({ 'X-CSRF-Token': session.csrf_token }) })));
});
it('keeps protected navigation hidden until the session has been checked', async () => {
  let resolve!: (value: Response) => void;
  vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((done) => { resolve = done; })));
  renderAt('/admin/users');
  expect(screen.queryByRole('link', { name: 'Пользователи' })).toBeNull();
  expect(screen.getByText('Проверяем сессию…')).toBeDefined();
  resolve(unauthorized());
  expect(await screen.findByRole('heading', { name: 'Вход в систему' })).toBeDefined();
  expect(screen.queryByRole('heading', { name: 'Пользователи' })).toBeNull();
});
it('restores an administrator on a nested route after reload', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(response(url.endsWith('/auth/me') ? session : url.endsWith('/health') ? {status: 'ok', service: 'veruna-releases', version: '2.3.4'} : [user]))));
  renderAt('/admin/users');
  expect(await screen.findByRole('heading', { name: 'Пользователи' })).toBeDefined();
  expect(await screen.findByRole('button', { name: /Добавить пользователя/ })).toBeDefined();
  expect(await screen.findByRole('button', { name: 'Действия: owner' })).toBeDefined();
  expect(await screen.findByText('Версия 2.3.4')).toBeDefined();
});
it('hides user management for an operator and denies its direct route', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(response(url.endsWith('/health') ? {status: 'ok', service: 'veruna-releases', version: '2.3.4'} : { ...session, user: { ...user, role: 'operator' } })));
  vi.stubGlobal('fetch', fetch); renderAt('/admin/users');
  expect(await screen.findByText('Доступ ограничен')).toBeDefined();
  expect(screen.queryByRole('link', { name: 'Пользователи' })).toBeNull();
  expect(fetch.mock.calls.some(([url]) => url.startsWith('/api/v1/admin/'))).toBe(false);
});
it('returns to login when the session expires', async () => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(response(session)))); renderAt('/admin/releases');
  expect(await screen.findByRole('heading', { name: 'Релизы' })).toBeDefined();
  fireEvent(window, new Event('session-expired'));
  expect(await screen.findByRole('heading', { name: 'Вход в систему' })).toBeDefined();
  expect(screen.queryByRole('heading', { name: 'Релизы' })).toBeNull();
});
it('shows a session verification error with a retry', async () => {
  vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('offline'))); renderAt('/admin/profile');
  expect(await screen.findByText('Не удалось проверить сессию')).toBeDefined();
  expect(screen.getByRole('button', { name: 'Повторить' })).toBeDefined();
});
it('logs in and restores the originally requested page', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(url.endsWith('/auth/me') ? unauthorized() : response(session)));
  vi.stubGlobal('fetch', fetch); renderAt('/admin/releases');
  await screen.findByRole('heading', { name: 'Вход в систему' });
  fireEvent.change(screen.getByLabelText('Логин'), { target: { value: 'owner' } });
  fireEvent.change(screen.getByLabelText('Пароль'), { target: { value: 'password-12345' } });
  fireEvent.click(screen.getByRole('button', { name: 'Войти' }));
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Релизы' })).toBeDefined());
});
it('shows an invalid invitation error without a password form', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(url.endsWith('/auth/me') ? unauthorized() : response({ error: { code: 'invalid_token' } }, 400))));
  renderAt('/admin/activate#token=expired');
  expect(await screen.findByText('Не удалось открыть ссылку')).toBeDefined();
  expect(screen.queryByLabelText('Пароль')).toBeNull();
});
