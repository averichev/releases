import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { App } from 'antd';
import { MemoryRouter } from 'react-router-dom';
import { expect, it, vi } from 'vitest';
import { AppRoutes } from '../App';
import { detectedType, size, validVersion } from '../api/releases';
const application = { download_count: 0, id: 'b8daf7db-a749-4165-a559-860ddbc6f28c', name: 'Product', slug: 'product', description: '', icon_url: null, visibility: 'private', status: 'active', created_at: 1, updated_at: 1 };
const release = { download_count: 0, id: '8d643b6a-6b79-483a-b819-c2617ff6be27', application_id: application.id, version: '1.2.0', title: null, release_notes: 'Changes', status: 'draft', created_at: 1, updated_at: 1, published_at: null, artifact_count: 1, total_size_bytes: 3 };
const artifact = { download_count: 0, id: 'file-id', release_id: release.id, filename: 'installer.bin', target: 'windows-x86_64', file_type: 'bin', size_bytes: 3, sha256: 'a'.repeat(64), created_at: 1 };
const options = { targets: ['windows-x86_64', 'any'], file_types: ['bin', 'exe'], max_file_bytes: 1024, upload_idle_seconds: 120 };
const user = { id: 'owner', username: 'owner', role: 'admin', status: 'active', created_at: 1, updated_at: 1, last_login_at: 1 };
const session = { user, csrf_token: 'csrf' };
const route = `/admin/applications/${application.id}/releases/${release.id}`;
const response = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status });
function renderAt(path: string) { return render(<App><MemoryRouter initialEntries={[path]}><AppRoutes /></MemoryRouter></App>); }
function mockFetch(role = 'admin') {
  const fetch = vi.fn((url: string, init?: RequestInit) => Promise.resolve(response(url.endsWith('/auth/me') ? { ...session, user: { ...user, role } }
    : url.endsWith('/artifact-options') ? options : url.endsWith('/artifacts') ? [artifact] : url.includes('/releases?') ? { items: [release], total: 1, page: 1, per_page: 20 }
      : url.endsWith(`/${release.id}`) || init?.method === 'POST' ? release : application, init?.method === 'POST' ? 201 : 200)));
  vi.stubGlobal('fetch', fetch); return fetch;
}
it('validates SemVer, compound extensions and binary size units', () => {
  for (const value of ['1.0.0', ' v1.2.0 ', '2.0.0-rc.1', '1.0.0+build.1']) expect(validVersion(value)).toBe(true);
  for (const value of ['1.0', '01.0.0', '1.0.0-01', '1.0.0+']) expect(validVersion(value)).toBe(false);
  expect(detectedType('installer.TAR.GZ')).toBe('tar.gz'); expect(detectedType('installer')).toBeUndefined();
  expect(size(1024)).toBe('1 KiB'); expect(size(2 * 1024 ** 3)).toBe('2 GiB');
});
it('creates a draft and opens its nested detail route', async () => {
  const fetch = mockFetch(); renderAt(`/admin/applications/${application.id}/releases`);
  fireEvent.click(await screen.findByRole('button', { name: 'Создать релиз' }));
  fireEvent.change(await screen.findByLabelText('Версия'), { target: { value: '1.2.0' } });
  fireEvent.click(screen.getByRole('button', { name: 'Создать черновик' }));
  expect(await screen.findByRole('heading', { name: '1.2.0' }, { timeout: 4000 })).toBeDefined();
  expect(await screen.findByText('installer.bin')).toBeDefined();
  const request = fetch.mock.calls.find(([, init]) => init?.method === 'POST');
  expect(JSON.parse(request?.[1]?.body as string)).toMatchObject({ version: '1.2.0' });
  expect(request?.[1]?.headers).toMatchObject({ 'X-CSRF-Token': 'csrf' });
}, 10000);
it('shows server version conflicts inside the creation form', async () => {
  const fetch = mockFetch(); fetch.mockImplementation((url: string, init?: RequestInit) => Promise.resolve(init?.method === 'POST' ? response({ error: { code: 'release_version_exists' } }, 409) : response(url.endsWith('/auth/me') ? session : url.includes('/releases?') ? { items: [], total: 0, page: 1, per_page: 20 } : application)));
  renderAt(`/admin/applications/${application.id}/releases`); fireEvent.click(await screen.findByRole('button', { name: 'Создать релиз' }));
  fireEvent.change(await screen.findByLabelText('Версия'), { target: { value: '1.2.0' } }); fireEvent.click(screen.getByRole('button', { name: 'Создать черновик' }));
  expect(await screen.findByText(/Релиз с этой версией уже существует/)).toBeDefined();
});
it('restores detail for operators with download and checksum but no mutations', async () => {
  mockFetch('operator'); renderAt(route);
  expect(await screen.findByRole('heading', { name: '1.2.0' })).toBeDefined();
  expect(screen.getByRole('link', { name: 'Скачать' }).getAttribute('href')).toBe(`/api/v1/admin/applications/${application.id}/releases/${release.id}/artifacts/file-id/download`);
  expect(screen.getByText('a'.repeat(64))).toBeDefined();
  expect(screen.queryByRole('button', { name: 'Добавить артефакт' })).toBeNull(); expect(screen.queryByRole('button', { name: 'Удалить черновик' })).toBeNull();
});
it('edits draft metadata and requires confirmation before deletion', async () => {
  const fetch = mockFetch(); renderAt(route);
  fireEvent.click(await screen.findByRole('button', { name: 'Изменить релиз' })); fireEvent.change(await screen.findByLabelText('Описание изменений'), { target: { value: 'Updated' } });
  fireEvent.click(screen.getByRole('button', { name: 'Сохранить релиз' }));
  await waitFor(() => expect(fetch.mock.calls.some(([, init]) => init?.method === 'PATCH' && (init.body as string).includes('Updated'))).toBe(true));
  await waitFor(() => expect(screen.queryByLabelText('Описание изменений')).toBeNull());
  fireEvent.click(screen.getByRole('button', { name: 'Удалить черновик' }));
  expect(await screen.findByText('Черновик и все его файлы будут удалены. Восстановить их через панель нельзя.')).toBeDefined();
  expect(fetch.mock.calls.some(([, init]) => init?.method === 'DELETE')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'Отмена' }));
});
it('shows detail errors and allows retry', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(url.endsWith('/auth/me') ? response(session) : response({ error: { code: 'internal_error' } }, 500))));
  renderAt(route); expect(await screen.findByText('Не удалось загрузить релиз')).toBeDefined(); expect(screen.getByRole('button', { name: 'Повторить' })).toBeDefined();
});

it('keeps active uploads open on Escape and supports explicit cancellation', async () => {
  class PendingXhr {
    upload = { onprogress: null as ((event: { lengthComputable: boolean; loaded: number; total: number }) => void) | null };
    onabort?: () => void;
    open() {} setRequestHeader() {} send() {} abort() { this.onabort?.(); }
  }
  vi.stubGlobal('XMLHttpRequest', PendingXhr); mockFetch(); renderAt(route);
  fireEvent.click(await screen.findByRole('button', { name: 'Добавить артефакт' }));
  fireEvent.mouseDown(await screen.findByRole('combobox', { name: 'Тип артефакта' }));
  fireEvent.click(await screen.findByText('Архив', {selector: '.ant-select-item-option-content'}));
  const platform = await screen.findByRole('combobox', { name: 'Целевая платформа' }); fireEvent.mouseDown(platform);
  fireEvent.click(await screen.findByText('Любая платформа'));
  const input = document.querySelector('input[type="file"]') as HTMLInputElement;
  fireEvent.change(input, { target: { files: [new File(['new'], 'new.zip')] } });
  expect(await screen.findByText('new.zip · 3 B · ZIP')).toBeDefined();
  const dialog = screen.getByRole('dialog', { name: 'Загрузка дистрибутива' });
  fireEvent.click(dialog.querySelector('button.ant-btn-primary')!);
  const cancel = await within(dialog).findByRole('button', { name: 'Отменить загрузку' });
  fireEvent.keyDown(dialog, { key: 'Escape', code: 'Escape', keyCode: 27 });
  expect(screen.getByRole('dialog', { name: 'Загрузка дистрибутива' })).toBeDefined();
  expect(within(dialog).getByRole('button', { name: 'Отменить загрузку' })).toBeDefined();
  fireEvent.click(cancel);
  expect(await screen.findByText(/Загрузка отменена/)).toBeDefined();
  expect(within(dialog).queryByRole('button', { name: 'Отменить загрузку' })).toBeNull();
  expect(within(dialog).getByRole('button', { name: /close|закрыть/i })).toBeDefined();
}, 40000);

it('refreshes artifacts and public links after appending to a published release', async () => {
  const added = { ...artifact, id: 'updater-id', filename: 'updater.exe', file_type: 'exe', distribution_role: 'updater', signature_present: true };
  let uploaded = false;
  class AcceptedXhr {
    upload = {}; status = 201; responseText = JSON.stringify(added); onload?: () => void;
    open() {} setRequestHeader() {} send() { uploaded = true; this.onload?.(); }
  }
  vi.stubGlobal('XMLHttpRequest', AcceptedXhr);
  const fetch = vi.fn((url: string) => Promise.resolve(response(url.endsWith('/auth/me') ? session
    : url.endsWith('/artifact-options') ? options : url.endsWith('/artifacts') ? (uploaded ? [artifact, added] : [artifact])
      : url.includes('/distribution/releases/') ? { public_available: true, warnings: [], files: uploaded ? [{ id: added.id, url: 'https://example.test/new-updater', tauri_compatible: true, signature_present: true }] : [] }
        : url.endsWith(`/${release.id}`) ? { ...release, status: 'published', revision: uploaded ? 2 : 1, published_at: 1 } : application)));
  vi.stubGlobal('fetch', fetch); renderAt(route);
  fireEvent.click(await screen.findByRole('button', { name: 'Добавить артефакт' }));
  const dialog = await screen.findByRole('dialog', { name: 'Загрузка дистрибутива' });
  fireEvent.mouseDown(within(dialog).getByRole('combobox', { name: 'Тип артефакта' }));
  fireEvent.click(await screen.findByText('Пакет обновления (Tauri Updater)'));
  fireEvent.mouseDown(within(dialog).getByRole('combobox', { name: 'Целевая платформа' }));
  fireEvent.click(await screen.findByText('Windows x64', { selector: '.ant-select-item-option-content' }));
  fireEvent.change(dialog.querySelector('input[type="file"]')!, { target: { files: [new File(['new'], 'updater.exe')] } });
  await screen.findByText('updater.exe · 3 B · EXE');
  fireEvent.change(within(dialog).getByLabelText('Подпись Tauri (.sig)'), { target: { files: [new File(['signature'], 'updater.sig')] } });
  fireEvent.click(within(dialog).getByRole('button', { name: 'Загрузить файл' }));
  expect(await screen.findByText('Файл загружен и сохранён.')).toBeDefined();
  expect(await screen.findByText('updater.exe', { selector: '.artifact-filename' })).toBeDefined();
  expect(await screen.findByRole('link', { name: 'https://example.test/new-updater' })).toBeDefined();
  expect(screen.queryByRole('button', { name: 'Удалить' })).toBeNull();
  expect(screen.queryByRole('combobox', { name: 'Тип артефакта updater.exe' })).toBeNull();
}, 40000);

it('shows separate release and artifact download counts and refreshes them from the API', async () => {
  let refreshed = false;
  const fetch = vi.fn((url: string) => Promise.resolve(response(url.endsWith('/auth/me') ? { ...session, user: { ...user, role: 'operator' } }
    : url.endsWith('/artifact-options') ? options : url.endsWith('/artifacts') ? [{ ...artifact, download_count: refreshed ? 18 : 17 }]
      : url.endsWith(`/${release.id}`) ? { ...release, download_count: refreshed ? 43 : 42 } : application)));
  vi.stubGlobal('fetch', fetch); renderAt(route);
  await screen.findByRole('heading', { name: release.version });
  expect(within(document.querySelector('.release-info')!).getByText('42')).toBeDefined();
  expect(within(document.querySelector('.release-artifacts')!).getByText('17')).toBeDefined();
  refreshed = true;
  fireEvent.click(screen.getByRole('button', { name: 'Обновить список' }));
  expect(await within(document.querySelector('.release-info')!).findByText('43')).toBeDefined();
  expect(within(document.querySelector('.release-artifacts')!).getByText('18')).toBeDefined();
});
