import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { App } from 'antd';
import { expect, it, vi } from 'vitest';
import { AppRoutes } from '../App';
const artifact = { id: 'file-1', filename: 'setup.exe', target: 'windows-x86_64', format: 'exe', purpose: 'installer', size_bytes: 1024, download_url: 'https://apps.example.test/product/releases/1.0.0/artifacts/file-1/download' };
const release = { version: '1.0.0', published_at: '2026-11-12T10:00:00Z', notes: 'Исправления', artifacts: [artifact] };
const application = { slug: 'product', name: 'Product', description: 'Описание продукта', icon_url: null, manifest_url: 'https://apps.example.test/product/manifest.json', latest_by_target: {'windows-x86_64': release} };
function renderPage() {render(<App><MemoryRouter initialEntries={['/product']}><AppRoutes /></MemoryRouter></App>);}
it('loads public data without a session and exposes a real anchor to the file', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.endsWith('/health') ? {status: 'ok', service: 'veruna-releases', version: '2.3.4'} : url.includes('/releases?') ? {items: [release], total: 1} : application))));
  vi.stubGlobal('fetch', fetch);renderPage();
  expect(await screen.findByRole('heading', {name: 'Product'})).toBeDefined();
  expect((await screen.findByRole('link', {name: /Скачать EXE/})).getAttribute('href')).toBe(artifact.download_url);
  expect(await screen.findByText('Версия 2.3.4')).toBeDefined();
  expect(screen.queryByRole('navigation', {name: 'Основная навигация'})).toBeNull();
  expect(fetch.mock.calls.every(([url]) => !url.includes('/auth/'))).toBe(true);
});
it('opens and copies the server-generated manifest URL', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.includes('/releases?') ? {items: [], total: 0} : application)))));
  const copy = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal('navigator', {clipboard: {writeText: copy}});
  renderPage();
  expect((await screen.findByRole('link', {name: 'Манифест JSON'})).getAttribute('href')).toBe(application.manifest_url);
  fireEvent.click(screen.getByRole('button', {name: 'Скопировать ссылку на манифест'}));
  await waitFor(() => expect(copy).toHaveBeenCalledWith(application.manifest_url));
  expect(await screen.findByText('Ссылка на манифест скопирована')).toBeDefined();
});
it('reports denied clipboard access and keeps the manifest link available', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.includes('/releases?') ? {items: [], total: 0} : application)))));
  vi.stubGlobal('navigator', {clipboard: {writeText: vi.fn().mockRejectedValue(new Error('denied'))}});
  renderPage();
  fireEvent.click(await screen.findByRole('button', {name: 'Скопировать ссылку на манифест'}));
  expect(await screen.findByText('Не удалось скопировать. Скопируйте адрес через меню ссылки.')).toBeDefined();
  expect(screen.queryByText('Ссылка на манифест скопирована')).toBeNull();
  expect(screen.getByRole('link', {name: 'Манифест JSON'}).getAttribute('href')).toBe(application.manifest_url);
});
it('shows current installers of different versions for each platform', async () => {
  const mac = {...artifact, id: 'mac', target: 'darwin-aarch64', filename: 'product.dmg', format: 'dmg', download_url: 'https://apps.example.test/product/releases/1.1.0/artifacts/mac/download'};
  const data = {...application, latest_by_target: {'windows-x86_64': release, 'darwin-aarch64': {...release, version: '1.1.0', artifacts: [mac]}}};
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.includes('/releases?') ? {items: [], total: 0} : data)))));
  renderPage();
  expect((await screen.findByRole('link', {name: /Скачать EXE/})).getAttribute('href')).toBe(artifact.download_url);
  expect(screen.getByRole('link', {name: /Скачать DMG/}).getAttribute('href')).toBe(mac.download_url);
  expect(screen.getByText('v1.0.0')).toBeDefined(); expect(screen.getByText('v1.1.0')).toBeDefined();
});
it('shows a real empty release state', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.includes('/releases?') ? {items: [], total: 0} : {...application, latest_by_target: {}})))));
  renderPage();expect(await screen.findByText('Файлов для скачивания пока нет')).toBeDefined();
});
it('shows 404 for a hidden application', async () => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response(JSON.stringify({error: {code: 'not_found'}}), {status: 404}))));
  renderPage();expect(await screen.findByText('Приложение не найдено')).toBeDefined();
  expect(screen.queryByText('Описание продукта')).toBeNull();
  expect(screen.queryByRole('link', {name: 'Манифест JSON'})).toBeNull();
  expect(screen.queryByRole('button', {name: 'Скопировать ссылку на манифест'})).toBeNull();
});

it('shows standalone binaries and archives without requiring any installer', async () => {
  const binary = {...artifact, filename: 'mockport', target: 'linux-x86_64', format: 'bin', purpose: 'executable'};
  const archive = {...binary, id: 'archive', filename: 'mockport.tar.gz', format: 'tar.gz', purpose: 'archive'};
  const binaryRelease = {...release, artifacts: [binary]};
  const archiveRelease = {...release, artifacts: [archive]};
  const data = {...application, latest_by_target: {}, latest_by_type: {installer: {}, executable: {'linux-x86_64': binaryRelease}, archive: {'linux-x86_64': archiveRelease}}};
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.includes('/releases?') ? {items: [{...release, artifacts: [binary, archive]}], total: 1} : data)))));
  renderPage();
  expect((await screen.findByRole('link', {name: /Скачать BIN/})).getAttribute('href')).toBe(binary.download_url);
  expect(screen.getByRole('link', {name: /Скачать TAR.GZ/})).toBeDefined();
  expect(screen.getByText('Linux x64 · Исполняемый файл')).toBeDefined();
  expect(screen.getByText('Linux x64 · Архив')).toBeDefined();
  expect(screen.queryByText('Файлов для скачивания пока нет')).toBeNull();
});
