import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { App } from 'antd';
import { expect, it, vi } from 'vitest';
import { AppRoutes } from '../App';
const artifact = { id: 'file-1', filename: 'setup.exe', target: 'windows-x86_64', format: 'exe', purpose: 'installer', size_bytes: 1024, download_url: 'https://apps.example.test/product/releases/1.0.0/artifacts/file-1/download' };
const release = { version: '1.0.0', published_at: '2026-11-12T10:00:00Z', notes: 'Исправления', artifacts: [artifact] };
const application = { slug: 'product', name: 'Product', description: 'Описание продукта', icon_url: null, latest_by_target: {'windows-x86_64': release} };
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
  renderPage();expect(await screen.findByText('Установщиков пока нет')).toBeDefined();
});
it('shows 404 for a hidden application', async () => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response(JSON.stringify({error: {code: 'not_found'}}), {status: 404}))));
  renderPage();expect(await screen.findByText('Приложение не найдено')).toBeDefined();
  expect(screen.queryByText('Описание продукта')).toBeNull();
});
