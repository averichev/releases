import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { App } from 'antd';
import { MemoryRouter } from 'react-router-dom';
import { expect, it, vi } from 'vitest';
import type { Application } from '../api/applications';
import type { TauriSettings } from '../api/tauri';
import { command, signer } from '../integrations/tauri';
import CodeBlock from '../integrations/CodeBlock';
import TauriIntegration from './TauriIntegration';
const application: Application = {id: 'app-id', slug: 'product', name: 'Продукт', description: '', visibility: 'public', status: 'active', created_at: 1, updated_at: 1, icon_url: null};
const endpoints = {stable: 'https://updates.example.test/api/v1/updates/tauri/product/stable/{{target}}/{{arch}}/{{current_version}}', beta: 'https://updates.example.test/api/v1/updates/tauri/product/beta/{{target}}/{{arch}}/{{current_version}}'};
function settings(key: string | null = null, channel: 'stable' | 'beta' = 'stable'): TauriSettings {
  return {public_key: key, public_base_url: 'https://updates.example.test', endpoint: endpoints[channel], endpoints, channel, current_version: null, published_release_exists: false, ready: false, packages: [], validation: null, full_integrity_checked: false,
    blockers: [{code: 'channel_release_missing', artifact_id: null, message: 'Нет текущего релиза канала.'}],
    configuration: key ? {bundle: {createUpdaterArtifacts: true}, plugins: {updater: {pubkey: key, endpoints: [endpoints[channel]]}}} : null};
}
const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), {status, headers: {'Content-Type': 'application/json'}});
function show(query = '', administrator = true) {
  return render(<App><MemoryRouter initialEntries={[`/admin/applications/app-id/integrations/tauri${query}`]}><TauriIntegration application={application} administrator={administrator} /></MemoryRouter></App>);
}
it('keeps missing-key configuration and readiness honest', async () => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(json(settings())))); show('?step=3');
  await screen.findByText('Не настроен публичный ключ');
  expect(screen.queryByRole('button', {name: 'Копировать Фрагмент tauri.conf.json'})).toBeNull();
  expect(screen.queryByText('Серверная конфигурация готова')).toBeNull();
});
it('saves the key through the protected API and reloads durable settings', async () => {
  let key: string | null = null;
  const fetch = vi.fn((_url: string, init?: RequestInit) => {
    if (init?.method === 'PUT') {key = (JSON.parse(String(init.body)) as {public_key: string}).public_key; return Promise.resolve(new Response(null, {status: 204}));}
    return Promise.resolve(json(settings(key)));
  });
  vi.stubGlobal('fetch', fetch); show('?step=2');
  fireEvent.change(await screen.findByRole('textbox', {name: 'Публичный ключ Tauri'}), {target: {value: 'public-test-key'}});
  fireEvent.click(screen.getByRole('button', {name: 'Сохранить публичный ключ'}));
  await screen.findByText('Публичный ключ сохранён');
  expect(fetch.mock.calls.some(([, init]) => init?.method === 'PUT' && String(init.body).includes('public-test-key'))).toBe(true);
  expect((await screen.findByRole('textbox', {name: 'Публичный ключ Tauri'}) as HTMLTextAreaElement).value).toBe('public-test-key');
});
it('copies the complete server-generated configuration including its key and placeholders', async () => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(json(settings('saved-key')))));
  const copy = vi.fn().mockResolvedValue(undefined); Object.defineProperty(navigator, 'clipboard', {configurable: true, value: {writeText: copy}});
  show('?step=3'); fireEvent.click(await screen.findByRole('button', {name: 'Копировать Фрагмент tauri.conf.json'}));
  await waitFor(() => expect(copy).toHaveBeenCalled());
  const copied = JSON.parse(copy.mock.calls[0]![0] as string) as {plugins: {updater: {pubkey: string; endpoints: string[]}}};
  expect(copied.plugins.updater.pubkey).toBe('saved-key'); expect(copied.plugins.updater.endpoints[0]).toBe(endpoints.stable);
});
it('restores step and beta from the URL and uses server-generated stable data when switching', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(json(settings('saved-key', url.includes('channel=beta') ? 'beta' : 'stable')))));
  show('?step=3&channel=beta');
  const block = await screen.findByLabelText('Фрагмент tauri.conf.json');
  expect(block.textContent).toContain(endpoints.beta);
  fireEvent.click(screen.getByText('Stable'));
  await waitFor(() => expect(screen.getByLabelText('Фрагмент tauri.conf.json').textContent).toContain(endpoints.stable));
});
it('shows available update details, its actual JSON and the public endpoint check', async () => {
  const response = {version: '1.0.1', url: 'https://updates.example.test/product/download', signature: 'test-signature', notes: '', pub_date: '2026-01-01T00:00:00Z'};
  const fetch = vi.fn((url: string, init?: RequestInit) => {
    if (url.startsWith('https://')) return Promise.resolve(json(response));
    if (url.endsWith('/preview')) {
      expect(JSON.parse(String(init?.body))).toEqual({channel: 'stable', target: 'windows-x86_64', current_version: '1.0.0'});
      return Promise.resolve(json({state: 'available', message: 'Обновление доступно.', reason: 'update_available', selected_version: '1.0.1', artifact_id: 'file-id', artifact_filename: 'product.exe', signature_present: true, public_downloads_available: true, response}));
    }
    return Promise.resolve(json(settings('key')));
  });
  vi.stubGlobal('fetch', fetch); show('?step=7');
  fireEvent.click(await screen.findByRole('button', {name: 'Проверить обновление'}));
  await screen.findByText('Сервер предлагает обновление');
  expect(screen.getByText('product.exe')).toBeDefined(); expect(screen.getByLabelText('JSON-ответ Tauri').textContent).toContain('1.0.1');
  await screen.findByText('Публичный endpoint отвечает');
  const call = fetch.mock.calls.find(([url]) => url.startsWith('https://'))!;
  expect(call[0]).toContain('/windows/x86_64/1.0.0'); expect(call[1]?.credentials).toBe('omit');
});
it('distinguishes a valid no-update preview from configuration and network errors', async () => {
  let fail = false;
  vi.stubGlobal('fetch', vi.fn((url: string) => {
    if (url.startsWith('https://')) return Promise.resolve(new Response(null, {status: 204}));
    if (url.endsWith('/preview')) return Promise.resolve(fail ? json({error: {code: 'invalid_release'}}, 422) : json({state: 'no_update', reason: 'version_not_newer', message: 'Версия канала не новее клиента.', selected_version: '1.0.0', artifact_id: null, signature_present: false, public_downloads_available: true, response: null}));
    return Promise.resolve(json(settings('key')));
  }));
  show('?step=7'); fireEvent.click(await screen.findByRole('button', {name: 'Проверить обновление'}));
  await screen.findByText('Обновления нет — протокол 204'); await screen.findByText('Публичный endpoint отвечает');
  fail = true; fireEvent.change(screen.getByLabelText('Текущая установленная версия'), {target: {value: 'invalid'}});
  fireEvent.click(screen.getByRole('button', {name: 'Проверить обновление'}));
  await screen.findByText('Не удалось проверить обновление'); expect(screen.queryByText('Обновления нет — протокол 204')).toBeNull();
});
it('allows operator instructions but hides the key mutation form and displays load errors', async () => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(json(settings())))); const view = show('?step=2', false);
  await screen.findByText('Сохранить публичный ключ может администратор'); expect(screen.queryByRole('button', {name: 'Сохранить публичный ключ'})).toBeNull();
  view.unmount(); vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('offline'))); show();
  await screen.findByText('Не удалось загрузить интеграцию'); expect(screen.getByRole('button', {name: 'Повторить'})).toBeDefined();
});
it('copies raw code, escapes HTML in highlighted snippets, and reports clipboard failure', async () => {
  const copy = vi.fn().mockRejectedValue(new Error('denied')); Object.defineProperty(navigator, 'clipboard', {configurable: true, value: {writeText: copy}});
  const code = JSON.stringify({key: '<script>alert(1)</script>'});
  const view = render(<App><CodeBlock title="Пример" language="json" code={code} /></App>);
  expect(view.container.querySelector('script')).toBeNull();
  fireEvent.click(screen.getByRole('button', {name: 'Копировать Пример'}));
  await screen.findByText('Не удалось скопировать. Выделите текст вручную.'); expect(copy).toHaveBeenCalledWith(code);
});
it('uses the selected package-manager CLI including npm signer argument forwarding', () => {
  expect(command('pnpm', 'add updater')).toBe('pnpm tauri add updater');
  expect(command('npm', 'add process')).toBe('npm run tauri add process');
  expect(command('yarn', 'build')).toBe('yarn tauri build');
  expect(command('bun', 'build')).toBe('bunx tauri build');
  expect(signer('npm', 'product-one')).toBe('npm run tauri signer generate -- -w ~/.tauri/product-one.key');
});
it('reports a failed public endpoint separately from the successful server preview', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => {
    if (url.startsWith('https://')) return Promise.reject(new Error('TLS/network failure'));
    if (url.endsWith('/preview')) return Promise.resolve(json({state: 'no_update', reason: 'empty_channel', message: 'Канал пуст.', selected_version: null, artifact_id: null, signature_present: false, public_downloads_available: true, response: null}));
    return Promise.resolve(json(settings('key')));
  }));
  show('?step=7'); fireEvent.click(await screen.findByRole('button', {name: 'Проверить обновление'}));
  await screen.findByText('Серверный endpoint недоступен или ответ не соответствует протоколу');
  expect(screen.queryByText('Публичный endpoint отвечает')).toBeNull();
});
