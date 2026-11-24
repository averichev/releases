import { afterEach, vi } from 'vitest';
import { cleanup, configure } from '@testing-library/react';

// Ant Design rendering/accessibility scans can exceed the default 1s on a busy
// CI host; keep async assertions inside the existing 30s per-test budget.
configure({ asyncUtilTimeout: 5000 });

Object.defineProperty(window, 'matchMedia', { writable: true, value: vi.fn().mockImplementation((query: string) => ({
  matches: false, media: query, onchange: null,
  addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
})) });

// jsdom has no layout engine; Ant Design uses these browser APIs for tables.
Object.defineProperty(window, 'ResizeObserver', { writable: true, value: class {
  observe() {} unobserve() {} disconnect() {}
} });
const getComputedStyle = window.getComputedStyle;
window.getComputedStyle = (element) => getComputedStyle(element);

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
