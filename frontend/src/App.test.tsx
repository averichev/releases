import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { expect, it, vi } from 'vitest';
import { AppRoutes } from './App';

it('opens a nested section and navigates without business data', () => {
  render(<MemoryRouter initialEntries={['/admin/releases']}><AppRoutes /></MemoryRouter>);
  expect(screen.getByRole('heading', { name: 'Релизы' })).toBeDefined();
  expect(screen.getByText('Раздел запланирован')).toBeDefined();
  fireEvent.click(screen.getByRole('link', { name: 'API-токены' }));
  expect(screen.getByRole('heading', { name: 'API-токены' })).toBeDefined();
});

it('shows a real failure instead of claiming that the service is ready', async () => {
  vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('Connection failed')));
  render(<MemoryRouter initialEntries={['/admin']}><AppRoutes /></MemoryRouter>);
  expect(await screen.findByText('Сервис недоступен')).toBeDefined();
  expect(screen.queryByText('Сервис готов к работе')).toBeNull();
});
