import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { App } from 'antd';
import { expect, it, vi } from 'vitest';
import { AppRoutes } from '../App';

const application = { name: 'Product', slug: 'product', icon_url: '/api/v1/public/applications/product/icon' };
const response = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status });
function renderPage() { render(<App><MemoryRouter initialEntries={['/']}><AppRoutes /></MemoryRouter></App>); }
const health = { status: 'ok', service: 'veruna-releases', version: '2.3.4' };

it('opens the public catalog without a session and links the name and logo to application details', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(response(url.endsWith('/health') ? health
    : url.includes('/releases?') ? { items: [], total: 0 }
      : url.endsWith('/product') ? { ...application, description: 'Описание', latest_by_target: {} }
        : { items: [application, { name: 'Without icon', slug: 'without-icon', icon_url: null }], total: 2 })));
  vi.stubGlobal('fetch', fetch); renderPage();
  const link = await screen.findByRole('link', { name: 'Product' });
  expect(link.getAttribute('href')).toBe('/product');
  expect(link.querySelector('img')?.getAttribute('src')).toBe(application.icon_url);
  expect(screen.getByRole('link', { name: /Without icon/ }).getAttribute('href')).toBe('/without-icon');
  expect(screen.queryByRole('navigation', { name: 'Основная навигация' })).toBeNull();
  fireEvent.click(link.querySelector('img')!);
  expect(await screen.findByRole('heading', { name: 'Product' })).toBeDefined();
  fireEvent.click(screen.getByRole('link', { name: 'Релизы — приложения' }));
  expect(await screen.findByRole('heading', { name: 'Приложения' })).toBeDefined();
  expect(fetch.mock.calls.every(([url]) => !url.includes('/auth/'))).toBe(true);
});

it('distinguishes loading from an empty catalog', async () => {
  let resolve!: (value: Response) => void;
  vi.stubGlobal('fetch', vi.fn((url: string) => url.endsWith('/health') ? Promise.resolve(response(health))
    : new Promise<Response>((done) => { resolve = done; })));
  renderPage();
  expect(await screen.findByLabelText('Загрузка приложений')).toBeDefined();
  expect(screen.queryByText('Опубликованных приложений пока нет')).toBeNull();
  resolve(response({ items: [], total: 0 }));
  expect(await screen.findByText('Опубликованных приложений пока нет')).toBeDefined();
});

it('retries a failed request', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(response(url.endsWith('/health') ? health : { error: { code: 'database_unavailable' } }, url.endsWith('/health') ? 200 : 503)));
  vi.stubGlobal('fetch', fetch); renderPage();
  expect(await screen.findByText('Не удалось загрузить приложения')).toBeDefined();
  expect(screen.queryByText('Опубликованных приложений пока нет')).toBeNull();
  fetch.mockImplementation((url: string) => Promise.resolve(response(url.endsWith('/health') ? health : { items: [application], total: 1 })));
  fireEvent.click(screen.getByRole('button', { name: 'Повторить' }));
  expect(await screen.findByRole('link', { name: 'Product' })).toBeDefined();
});

it('requests the selected catalog page', async () => {
  const fetch = vi.fn((url: string) => Promise.resolve(response(url.endsWith('/health') ? health
    : { items: [{ ...application, name: url.includes('offset=20') ? 'Second page' : 'First page' }], total: 21 })));
  vi.stubGlobal('fetch', fetch); renderPage();
  await screen.findByRole('link', { name: 'First page' });
  fireEvent.click(screen.getByTitle('2'));
  expect(await screen.findByRole('link', { name: 'Second page' })).toBeDefined();
  expect(fetch.mock.calls.some(([url]) => url === '/api/v1/public/applications?limit=20&offset=20')).toBe(true);
});
