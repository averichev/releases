import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { App } from 'antd';
import { MemoryRouter } from 'react-router-dom';
import { expect, it, vi } from 'vitest';
import { AppRoutes } from '../App';
import { suggestSlug, validSlug } from '../api/applications';
const user = { id: 'owner-id', username: 'owner', role: 'admin', status: 'active', created_at: 1, updated_at: 1, last_login_at: 1 };
const session = { user, csrf_token: 'csrf-value' };
const app = { id: 'b8daf7db-a749-4165-a559-860ddbc6f28c', name: 'Просто CRM', slug: 'prosto-crm', description: 'Система для команды', icon_url: null, visibility: 'private', status: 'active', created_at: 1, updated_at: 1 };
const response = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status });
function renderAt(path: string) { return render(<App><MemoryRouter initialEntries={[path]}><AppRoutes /></MemoryRouter></App>); }
it('suggests transliterated slugs and applies reserved and length rules', () => {
  expect(suggestSlug('Просто CRM')).toBe('prosto-crm'); expect(suggestSlug(' Ёж и Щука! ')).toBe('yozh-i-shchuka');
  for (const slug of ['admin', '-bad', 'good-', 'UPPER', 'a'.repeat(65)]) expect(validSlug(slug)).toBe(false);
  expect(validSlug('a')).toBe(true);
});
it('creates an application with a suggested slug and opens its detail page', async () => {
  const fetch = vi.fn((url: string, init?: RequestInit) => Promise.resolve(response(url.endsWith('/auth/me') ? session : app, init?.method === 'POST' ? 201 : 200)));
  vi.stubGlobal('fetch', fetch); renderAt('/admin/applications/new');
  fireEvent.change(await screen.findByLabelText('Название'), { target: { value: 'Просто CRM' } });
  expect((screen.getByLabelText('Slug') as HTMLInputElement).value).toBe('prosto-crm');
  fireEvent.change(screen.getByLabelText('Описание'), { target: { value: 'Система для команды' } });
  fireEvent.click(screen.getByRole('button', { name: 'Создать приложение' }));
  expect(await screen.findByRole('heading', { name: 'Просто CRM' }, { timeout: 4000 })).toBeDefined();
  const request = fetch.mock.calls.find(([, init]) => init?.method === 'POST');
  expect(JSON.parse(request?.[1]?.body as string)).toMatchObject({ slug: 'prosto-crm', name: 'Просто CRM', visibility: 'private' });
  expect(request?.[1]?.headers).toMatchObject({ 'X-CSRF-Token': 'csrf-value' });
}, 10000);
it('keeps a manually chosen slug while the name changes and rejects reserved slugs', async () => {
  const fetch = vi.fn(() => Promise.resolve(response(session))); vi.stubGlobal('fetch', fetch); renderAt('/admin/applications/new');
  fireEvent.change(await screen.findByLabelText('Название'), { target: { value: 'Первое имя' } });
  fireEvent.change(screen.getByLabelText('Slug'), { target: { value: 'admin' } });
  fireEvent.change(screen.getByLabelText('Название'), { target: { value: 'Новое имя' } });
  expect((screen.getByLabelText('Slug') as HTMLInputElement).value).toBe('admin');
  fireEvent.click(screen.getByRole('button', { name: 'Создать приложение' }));
  expect(await screen.findByText('Недопустимый или служебный slug. Дефис не может стоять в начале или конце.')).toBeDefined();
  expect(fetch).toHaveBeenCalledTimes(1);
});
it('restores a nested application route and loads real releases', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(response(url.endsWith('/auth/me') ? session : url.includes('/releases?') ? { items: [], total: 0, page: 1, per_page: 20 } : app)))); renderAt(`/admin/applications/${app.id}/releases`);
  expect(await screen.findByRole('heading', { name: app.name })).toBeDefined(); expect(await screen.findByText('Релизов пока нет')).toBeDefined(); expect(screen.queryByText('1.0.0')).toBeNull();
});
it('edits settings without submitting slug, then archives and restores', async () => {
  let stored = { ...app };
  const fetch = vi.fn((url: string, init?: RequestInit) => {
    if (url.endsWith('/auth/me')) return Promise.resolve(response(session));
    if (init?.method === 'PATCH') stored = { ...stored, ...JSON.parse(init.body as string) as Partial<typeof app> };
    if (url.endsWith('/archive')) stored = { ...stored, status: 'archived' };
    if (url.endsWith('/restore')) stored = { ...stored, status: 'active' };
    return Promise.resolve(response(stored));
  });
  vi.stubGlobal('fetch', fetch); renderAt(`/admin/applications/${app.id}/settings`);
  fireEvent.change(await screen.findByLabelText('Название'), { target: { value: 'Новое имя' } });
  expect((screen.getByLabelText('Slug') as HTMLInputElement).readOnly).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Сохранить изменения' }));
  expect(await screen.findByRole('heading', { name: 'Новое имя' })).toBeDefined();
  const patch = fetch.mock.calls.find(([, init]) => init?.method === 'PATCH'); expect(JSON.parse(patch?.[1]?.body as string)).not.toHaveProperty('slug');
  fireEvent.click(screen.getByRole('button', { name: 'Архивировать приложение' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Архивировать' }));
  expect(await screen.findByRole('button', { name: 'Восстановить приложение' })).toBeDefined();
  fireEvent.click(screen.getByRole('button', { name: 'Восстановить приложение' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Восстановить' }));
  expect(await screen.findByRole('button', { name: 'Архивировать приложение' })).toBeDefined();
}, 25000);
it('allows operators to view the catalog but hides creation', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(response(url.endsWith('/auth/me') ? { ...session, user: { ...user, role: 'operator' } } : { items: [app], total: 1, page: 1, per_page: 24 })));
  vi.stubGlobal('fetch', fetch); renderAt('/admin/applications');
  expect(await screen.findByRole('heading', { name: app.name })).toBeDefined(); expect(screen.queryByRole('button', { name: 'Создать приложение' })).toBeNull();
});
it('rejects the direct operator settings route', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(response(url.endsWith('/auth/me') ? { ...session, user: { ...user, role: 'operator' } } : app)))); renderAt(`/admin/applications/${app.id}/settings`);
  expect(await screen.findByText('Доступ ограничен')).toBeDefined(); expect(screen.queryByRole('button', { name: 'Сохранить изменения' })).toBeNull();
});
it('shows catalog errors and retries instead of reporting empty success', async () => {
  let failed = true;
  vi.stubGlobal('fetch', vi.fn((url: string) => url.endsWith('/auth/me') ? Promise.resolve(response(session)) : failed ? Promise.reject(new Error('offline')) : Promise.resolve(response({ items: [], total: 0, page: 1, per_page: 24 }))));
  renderAt('/admin/applications'); expect(await screen.findByText('Не удалось загрузить каталог')).toBeDefined(); expect(screen.queryByText('Приложений пока нет')).toBeNull();
  failed = false; fireEvent.click(screen.getByRole('button', { name: 'Повторить' })); expect(await screen.findByText('Приложений пока нет')).toBeDefined();
});
it('sends search to the server and resets pagination', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(response(url.endsWith('/auth/me') ? session : { items: [app], total: 1, page: 1, per_page: 24 })));
  vi.stubGlobal('fetch', fetch); renderAt('/admin/applications?page=2'); await screen.findByRole('heading', { name: app.name });
  const input = screen.getByLabelText('Поиск приложений'); fireEvent.change(input, { target: { value: 'CRM' } }); fireEvent.keyDown(input, { key: 'Enter', code: 'Enter', keyCode: 13 });
  await waitFor(() => expect(fetch.mock.calls.some(([url]) => url.includes('search=CRM') && url.includes('page=1'))).toBe(true));
});

it('preserves a created application when its icon upload fails', async () => {
  vi.stubGlobal('URL', class extends URL { static createObjectURL() { return 'blob:preview'; } static revokeObjectURL() {} });
  const fetch = vi.fn((url: string, init?: RequestInit) => Promise.resolve(url.endsWith('/auth/me') ? response(session)
    : url.endsWith('/icon') ? response({ error: { code: 'invalid_icon' } }, 400) : response(app, init?.method === 'POST' ? 201 : 200)));
  vi.stubGlobal('fetch', fetch); renderAt('/admin/applications/new');
  fireEvent.change(await screen.findByLabelText('Название'), { target: { value: app.name } });
  fireEvent.change(screen.getByLabelText('Файл иконки'), { target: { files: [new File(['bad'], 'icon.png', { type: 'image/png' })] } });
  fireEvent.click(screen.getByRole('button', { name: 'Создать приложение' }));
  expect(await screen.findByText(/Приложение создано, но иконка не сохранена/, {}, { timeout: 4000 })).toBeDefined();
  expect(screen.getByRole('button', { name: 'Сохранить изменения' })).toBeDefined();
  expect(fetch.mock.calls.filter(([url, init]) => url.endsWith('/applications') && init?.method === 'POST')).toHaveLength(1);
}, 10000);
