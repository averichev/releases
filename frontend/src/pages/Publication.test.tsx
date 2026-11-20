import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { App } from 'antd';
import { MemoryRouter } from 'react-router-dom';
import { expect, it, vi } from 'vitest';
import type { ReactNode } from 'react';
import ReleasePublication from './ReleasePublication';
import Manifests from './Manifests';
import type { Application } from '../api/applications';
import type { Release } from '../api/releases';
import { AppRoutes } from '../App';
const app: Application = { id: 'app', slug: 'product', name: 'Product', description: '', icon_url: null, status: 'active', visibility: 'public', created_at: 1, updated_at: 1 };
const release: Release = { id: 'release', application_id: app.id, version: '1.4.0', title: null, release_notes: 'Changes', status: 'draft', revision: 3, created_at: 1, updated_at: 1, published_at: null, artifact_count: 1, total_size_bytes: 3 };
const ready = { release_revision: 3, validation: { state: 'ready', warnings: [], errors: [] } };
const response = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status });
function show(element: ReactNode) { render(<App><MemoryRouter>{element}</MemoryRouter></App>); }
it('checks readiness, previews current and proposed versions and publishes only after confirmation', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(response(url.endsWith('/validate-publication') ? ready : release)));
  vi.stubGlobal('fetch', fetch); const onChange = vi.fn().mockResolvedValue(undefined);
  show(<ReleasePublication app={app.id} release={release} disabled={false} onChange={onChange} />);
  expect((screen.getByRole('button', { name: 'Опубликовать' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Проверить готовность' }));
  expect(await screen.findByText('Релиз готов к публикации')).toBeDefined();
  fireEvent.click(screen.getByRole('button', { name: 'Опубликовать' }));
  const modal = await screen.findByRole('dialog', { name: 'Подтверждение публикации' });
  expect(within(modal).getByText('1.4.0')).toBeDefined();
  expect(fetch.mock.calls.some(([url]) => url.endsWith('/publish'))).toBe(false);
  fireEvent.click(within(modal).getByRole('button', { name: 'Подтвердить публикацию' }));
  await waitFor(() => expect(fetch.mock.calls.some(([url]) => url.endsWith('/publish'))).toBe(true));
  const args = (fetch.mock.calls as unknown as [string, RequestInit][]).find(([url]) => url.endsWith('/publish'))!;
  expect(JSON.parse(args[1].body as string)).toEqual({ expected_release_revision: 3 });
  expect(await screen.findByText('Релиз опубликован')).toBeDefined();
});
it('shows blocking validation details and leaves publication disabled', async () => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(response({ ...ready, validation: { state: 'errors', warnings: [], errors: [{ code: 'checksum_mismatch', artifact_id: 'file', message: 'Контрольная сумма пакета не совпадает.' }] } }))));
  show(<ReleasePublication app={app.id} release={release} disabled={false} onChange={vi.fn().mockResolvedValue(undefined)} />);
  fireEvent.click(screen.getByRole('button', { name: 'Проверить готовность' }));
  expect(await screen.findByText('Контрольная сумма пакета не совпадает.')).toBeDefined();
  expect((screen.getByRole('button', { name: 'Опубликовать' }) as HTMLButtonElement).disabled).toBe(true);
});
it('reports concurrent publication changes and requires another readiness check', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(url.endsWith('/publish') ? response({ error: { code: 'publication_conflict' } }, 409) : response(ready))));
  show(<ReleasePublication app={app.id} release={release} disabled={false} onChange={vi.fn().mockResolvedValue(undefined)} />);
  fireEvent.click(screen.getByRole('button', { name: 'Проверить готовность' })); await screen.findByText('Релиз готов к публикации');
  fireEvent.click(screen.getByRole('button', { name: 'Опубликовать' })); fireEvent.click(await screen.findByRole('button', { name: 'Подтвердить публикацию' }));
  await waitFor(() => expect(screen.getAllByText(/Релиз изменён другим администратором/).length).toBeGreaterThan(0));
  expect((screen.getByRole('button', { name: 'Подтвердить публикацию' }) as HTMLButtonElement).disabled).toBe(true);
});
it('renders real manifest JSON and reports why Tauri cannot serve an update yet', async () => {
  const document = { schema_version: 2, app: 'product', latest_by_target: {}, releases: [] };
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(response(url.endsWith('/tauri/preview') ? { state: 'blocked', reason: 'signature_missing', message: 'Для обновления нужна подпись.', selected_version: '1.4.0', artifact_id: 'artifact', signature_present: true, response: null, public_downloads_available: false } : document))));
  show(<Manifests application={app} />);
  expect((await screen.findByLabelText('JSON манифеста')).textContent).toBe(JSON.stringify(document, null, 2));
  fireEvent.click(screen.getByRole('button', { name: 'Проверить Tauri' }));
  expect(await screen.findByText('Для обновления нужна подпись.')).toBeDefined();
  expect(screen.getByLabelText('Диагностика Tauri').textContent).toContain('"response": null');
});
it('allows appending to published releases while keeping existing files read only', async () => {
  const user = { id: 'owner', username: 'owner', role: 'admin', status: 'active' };
  const artifact = { id: 'artifact', release_id: release.id, filename: 'setup.exe', target: 'windows-x86_64', file_type: 'exe', size_bytes: 3, sha256: 'a'.repeat(64), created_at: 1, distribution_role: 'updater', signature_present: true };
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(response(url.endsWith('/auth/me') ? { user, csrf_token: 'csrf' } : url.includes('/distribution/releases/') ? {public_available: true, files: [{id: artifact.id, url: 'https://apps.example.test/product/releases/1.4.0/artifacts/artifact/download', tauri_compatible: true, signature_present: true}], warnings: []} : url.endsWith('/artifacts') ? [artifact] : url.endsWith('/artifact-options') ? { targets: [], file_types: [], max_file_bytes: 1024, upload_idle_seconds: 120 } : url.endsWith(`/${release.id}`) ? { ...release, status: 'published', published_at: 1 } : app))));
  render(<App><MemoryRouter initialEntries={[`/admin/applications/${app.id}/releases/${release.id}`]}><AppRoutes /></MemoryRouter></App>);
  expect(await screen.findByText('Published')).toBeDefined(); expect(screen.getByText('Подпись прикреплена')).toBeDefined();
  for (const name of ['Изменить релиз', 'Загрузить файл', 'Удалить', 'Удалить черновик', 'Опубликовать']) expect(screen.queryByRole('button', { name })).toBeNull();
  expect(screen.queryByRole('combobox', { name: 'Назначение setup.exe' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Добавить артефакт' }));
  expect(await screen.findByText('Релиз опубликован. После добавления файл нельзя будет изменить или удалить.')).toBeDefined();
});
