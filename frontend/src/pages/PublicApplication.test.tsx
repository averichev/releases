import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { App } from 'antd';
import { expect, it, vi } from 'vitest';
import { AppRoutes } from '../App';
const artifact = { id: 'file-1', filename: 'setup.exe', target: 'windows-x86_64', format: 'exe', purpose: 'installer', size_bytes: 1024, download_url: 'https://apps.example.test/product/releases/1.0.0/artifacts/file-1/download' };
const release = { version: '1.0.0', published_at: '2026-11-12T10:00:00Z', notes: 'Исправления', artifacts: [artifact] };
const application = { slug: 'product', name: 'Product', description: 'Описание продукта', icon_url: null, channel: 'stable', latest: release, default_artifact_id: 'file-1' };
function renderPage() {render(<App><MemoryRouter initialEntries={['/product']}><AppRoutes /></MemoryRouter></App>);}
it('loads public data without a session and exposes a real anchor to the file', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.includes('/releases?') ? {items: [release], total: 1} : application))));
  vi.stubGlobal('fetch', fetch);renderPage();
  expect(await screen.findByRole('heading', {name: 'Product'})).toBeDefined();
  expect((await screen.findByRole('link', {name: /Скачать Product/})).getAttribute('href')).toBe(artifact.download_url);
  expect(screen.queryByRole('navigation', {name: 'Основная навигация'})).toBeNull();
  expect(fetch.mock.calls.every(([url]) => !url.includes('/auth/'))).toBe(true);
});
it('does not choose arbitrarily when several installer platforms match', async () => {
  const ambiguous = {...application, default_artifact_id: null, latest: {...release, artifacts: [artifact, {...artifact, id: 'file-2', target: 'linux-x86_64', format: 'appimage'}]}};
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.includes('/releases?') ? {items: [], total: 0} : ambiguous)))));
  renderPage();expect(await screen.findByText('Выберите платформу и формат установщика.')).toBeDefined();
  expect(screen.queryByRole('link', {name: /Скачать Product/})).toBeNull();
});
it('shows a real empty release state', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.includes('/releases?') ? {items: [], total: 0} : {...application, latest: null})))));
  renderPage();expect(await screen.findByText('Стабильные релизы ещё не опубликованы')).toBeDefined();
});
it('shows 404 for a hidden application', async () => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response(JSON.stringify({error: {code: 'not_found'}}), {status: 404}))));
  renderPage();expect(await screen.findByText('Приложение не найдено')).toBeDefined();
  expect(screen.queryByText('Описание продукта')).toBeNull();
});
