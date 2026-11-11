import { afterEach, vi } from 'vitest';
import { cleanup } from '@testing-library/react';

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
