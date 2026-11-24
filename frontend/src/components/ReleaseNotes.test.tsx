import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import ReleaseNotes from './ReleaseNotes';

it('renders release Markdown and blocks embedded HTML, unsafe links and image requests', () => {
  const { container } = render(<ReleaseNotes>{'## Исправления\n- **Повтор** после обрыва.\n\n`curl`\n\n<script>alert(1)</script>\n\n[unsafe](javascript:alert%281%29)\n\n![tracking](https://example.test/pixel.png)'}</ReleaseNotes>);
  expect(screen.getByRole('heading', { name: 'Исправления', level: 2 })).toBeDefined();
  expect(container.querySelector('li strong')?.textContent).toBe('Повтор');
  expect(container.querySelector('code')?.textContent).toBe('curl');
  expect(container.querySelector('script')).toBeNull(); expect(container.querySelector('img')).toBeNull();
  expect(container.querySelector('a')?.getAttribute('href')).not.toContain('javascript:');
});
