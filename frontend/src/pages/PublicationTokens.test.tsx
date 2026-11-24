import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { App } from 'antd';
import { expect, it, vi } from 'vitest';
import { setCsrf } from '../api/client';
import PublicationTokens from './PublicationTokens';

const token = { id: 'token-id', name: 'CI Product', created_at: 1700000000, expires_at: null, state: 'active' };
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status });
function show() { return render(<App><PublicationTokens applicationId="app" archived={false} /></App>); }

it('creates through CSRF API, copies the one-time secret and clears it on close', async () => {
  const secret = 'vrp_one-time-secret'; let stored: typeof token[] = [];
  const fetch = vi.fn((_url: string, init?: RequestInit) => {
    if (init?.method === 'POST') { stored = [token]; return Promise.resolve(json({ ...token, token: secret }, 201)); }
    return Promise.resolve(json(stored));
  });
  const copy = vi.fn(() => Promise.resolve());
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: copy } });
  vi.stubGlobal('fetch', fetch); setCsrf('tokens-csrf'); show();
  const name = await screen.findByLabelText('Имя токена');
  fireEvent.change(name, { target: { value: token.name } });
  fireEvent.submit(name.closest('form')!);
  expect((await screen.findByLabelText('Токен публикации', {}, { timeout: 5000 }) as HTMLTextAreaElement).value).toBe(secret);
  const request = fetch.mock.calls.find(([, init]) => init?.method === 'POST');
  expect(request?.[0]).toBe('/api/v1/admin/applications/app/publication-tokens');
  expect(JSON.parse(String(request?.[1]?.body))).toEqual({ name: token.name, expires_at: null });
  expect(request?.[1]?.headers).toMatchObject({ 'X-CSRF-Token': 'tokens-csrf', 'X-Veruna-Request': '1' });
  fireEvent.click(screen.getByText('Скопировать токен'));
  await waitFor(() => expect(copy).toHaveBeenCalledWith(secret));
  fireEvent.click(screen.getByText('Готово'));
  await waitFor(() => expect(screen.queryByLabelText('Токен публикации')).toBeNull());
  expect(await screen.findByText(token.name)).toBeDefined();
  expect(localStorage.getItem('RELEASES_TOKEN')).toBeNull(); setCsrf();
});

it('revokes a token and refreshes its state without removing the row', async () => {
  let state = 'active';
  const fetch = vi.fn((_url: string, init?: RequestInit) => {
    if (init?.method === 'POST') { state = 'revoked'; return Promise.resolve(new Response(null, { status: 204 })); }
    return Promise.resolve(json([{ ...token, state }]));
  });
  vi.stubGlobal('fetch', fetch); show();
  fireEvent.click(await screen.findByText('Отозвать'));
  fireEvent.click(within(await screen.findByRole('dialog')).getByText('Отозвать'));
  expect(await screen.findByText('Отозван', {}, { timeout: 5000 })).toBeDefined(); expect(screen.getByText(token.name)).toBeDefined();
  expect(fetch.mock.calls.some(([url, init]) => url.endsWith('/token-id/revoke') && init?.method === 'POST')).toBe(true);
});

it('shows loading failures and allows retry before token creation', async () => {
  let failed = true;
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(failed ? json({ error: { code: 'internal_error' } }, 500) : json([token, { ...token, id: 'second', name: 'Replacement' }]))));
  show(); await screen.findByText('Не удалось загрузить токены');
  expect(screen.queryByText('Создать токен')).toBeNull();
  failed = false; fireEvent.click(screen.getByText('Повторить'));
  expect(await screen.findByText('Replacement')).toBeDefined();
  expect((screen.getByLabelText('Имя токена') as HTMLInputElement).disabled).toBe(true);
  expect((screen.getByText('Создать токен').closest('button') as HTMLButtonElement).disabled).toBe(true);
});
