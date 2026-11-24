import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { App } from 'antd';
import { expect, it, vi } from 'vitest';
import StandaloneSettings from './StandaloneSettings';
import { setCsrf } from '../api/client';
import { standalonePublicKey } from '../api/standalone';

const product = '00000000-0000-4000-8000-000000000001';
const publicKey = '11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo';
const empty = {application_id: 'app', product_id: null as string | null, public_key: null as string | null, key_id: null as string | null, product_id_locked: false, public_key_locked: false};
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), {status});
function show() { return render(<App><StandaloneSettings applicationId="app" /></App>); }

it('saves the product UUID and normalizes a public JWK through the protected API', async () => {
  let stored = {...empty};
  const fetch = vi.fn((_url: string, init?: RequestInit) => {
    if (init?.method === 'PUT') { stored = {...stored, ...JSON.parse(String(init.body)) as Partial<typeof stored>}; return Promise.resolve(new Response(null, {status: 204})); }
    return Promise.resolve(json(stored));
  });
  vi.stubGlobal('fetch', fetch); setCsrf('settings-csrf'); show();
  fireEvent.change(await screen.findByRole('textbox', {name: 'UUID продукта'}), {target: {value: product}});
  fireEvent.change(screen.getByRole('textbox', {name: 'Публичный ключ Ed25519'}), {target: {value: JSON.stringify({kty: 'OKP', crv: 'Ed25519', x: publicKey})}});
  fireEvent.click(screen.getByRole('button', {name: 'Сохранить настройки обновлений'}));
  await screen.findByText('Настройки подписанных обновлений сохранены');
  const request = fetch.mock.calls.find(([, init]) => init?.method === 'PUT');
  expect(request?.[0]).toBe('/api/v1/admin/applications/app/integrations/standalone');
  expect(JSON.parse(String(request?.[1]?.body))).toEqual({product_id: product, public_key: publicKey});
  expect(request?.[1]?.headers).toMatchObject({'X-CSRF-Token': 'settings-csrf', 'X-Veruna-Request': '1'});
  await waitFor(() => expect((screen.getByRole('textbox', {name: 'Публичный ключ Ed25519'}) as HTMLTextAreaElement).value).toBe(publicKey));
  setCsrf();
});

it('rejects private keys and invalid UUIDs without sending them to the server', async () => {
  const fetch = vi.fn(() => Promise.resolve(json(empty))); vi.stubGlobal('fetch', fetch); show();
  fireEvent.change(await screen.findByRole('textbox', {name: 'UUID продукта'}), {target: {value: 'not-a-uuid'}});
  fireEvent.change(screen.getByRole('textbox', {name: 'Публичный ключ Ed25519'}), {target: {value: JSON.stringify({kty: 'OKP', crv: 'Ed25519', x: publicKey, d: 'private-material'})}});
  fireEvent.click(screen.getByRole('button', {name: 'Сохранить настройки обновлений'}));
  await screen.findByText(/Приватный ключ не принимается/);
  expect(screen.getByText(/Введите UUID в строчном формате/)).toBeDefined();
  expect(fetch.mock.calls).toHaveLength(1);
});

it('preserves a concurrently changed key when saving only the UUID', async () => {
  let stored = {...empty, public_key: publicKey};
  const changedKey = 'PUAXw-hDiVqStwqnTRt-vJyYLM8uxJaMwM1V8Sr0Zgw';
  const fetch = vi.fn((_url: string, init?: RequestInit) => {
    if (init?.method === 'PUT') { stored = {...stored, ...JSON.parse(String(init.body)) as Partial<typeof stored>}; return Promise.resolve(new Response(null, {status: 204})); }
    return Promise.resolve(json(stored));
  });
  vi.stubGlobal('fetch', fetch); show();
  fireEvent.change(await screen.findByRole('textbox', {name: 'UUID продукта'}), {target: {value: product}});
  stored = {...stored, public_key: changedKey};
  fireEvent.click(screen.getByRole('button', {name: 'Сохранить настройки обновлений'}));
  await screen.findByText('Настройки подписанных обновлений сохранены');
  const request = fetch.mock.calls.find(([, init]) => init?.method === 'PUT');
  expect(JSON.parse(String(request?.[1]?.body))).toEqual({product_id: product});
  await waitFor(() => expect((screen.getByRole('textbox', {name: 'Публичный ключ Ed25519'}) as HTMLTextAreaElement).value).toBe(changedKey));
});

it('shows pinned values as read-only', async () => {
  const stored = {...empty, product_id: product, public_key: publicKey, product_id_locked: true, public_key_locked: true};
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(json(stored)))); show();
  expect((await screen.findByRole('textbox', {name: 'UUID продукта'}) as HTMLInputElement).readOnly).toBe(true);
  expect((screen.getByRole('textbox', {name: 'Публичный ключ Ed25519'}) as HTMLTextAreaElement).readOnly).toBe(true);
  expect((screen.getByRole('button', {name: 'Сохранить настройки обновлений'}) as HTMLButtonElement).disabled).toBe(true);
});

it.each(['standalone_product_conflict', 'standalone_key_conflict', 'forbidden'])('explains rejected save %s and preserves edits', async (code) => {
  const fetch = vi.fn((_url: string, init?: RequestInit) => Promise.resolve(init?.method === 'PUT' ? json({error: {code}}, code === 'forbidden' ? 403 : 409) : json(empty)));
  vi.stubGlobal('fetch', fetch); show();
  fireEvent.change(await screen.findByRole('textbox', {name: 'UUID продукта'}), {target: {value: product}});
  fireEvent.click(screen.getByRole('button', {name: 'Сохранить настройки обновлений'}));
  await waitFor(() => expect(screen.getAllByRole('alert').some((alert) => /другим приложением|Недостаточно прав/.test(alert.textContent ?? ''))).toBe(true));
  expect((screen.getByRole('textbox', {name: 'UUID продукта'}) as HTMLInputElement).value).toBe(product);
  expect(screen.queryByText('Настройки подписанных обновлений сохранены')).toBeNull();
});

it('retries loading without presenting missing data as successful configuration', async () => {
  let failed = true;
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(failed ? json({error: {code: 'internal_error'}}, 500) : json(empty)))); show();
  await screen.findByText('Не удалось загрузить настройки подписанных обновлений');
  expect(screen.queryByRole('textbox', {name: 'UUID продукта'})).toBeNull();
  failed = false; fireEvent.click(screen.getByRole('button', {name: 'Повторить'}));
  expect(await screen.findByRole('textbox', {name: 'UUID продукта'})).toBeDefined();
});

it.each(['-----BEGIN PRIVATE KEY-----', JSON.stringify({kty: 'RSA', crv: 'Ed25519', x: publicKey}), '{broken'])('rejects unsupported key input %s', (value) => {
  expect(() => standalonePublicKey(value)).toThrow();
});
