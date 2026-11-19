import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import ArtifactUploader from './ArtifactUploader';

it('explains a proxy upload limit and keeps the selected file for retry', async () => {
  class RejectedXhr {
    upload = {}; status = 413; responseText = '<html>413 Request Entity Too Large</html>';
    onload?: () => void;
    open() {} setRequestHeader() {} send() { this.onload?.(); }
  }
  vi.stubGlobal('XMLHttpRequest', RejectedXhr);
  const onChange = vi.fn(); const onBusy = vi.fn();
  render(<ArtifactUploader applicationId="application" releaseId="release" artifacts={[]} options={{
    targets: ['linux-x86_64'], file_types: ['deb'], max_file_bytes: 2147483648, upload_idle_seconds: 120,
  }} onChange={onChange} onBusy={onBusy} />);
  fireEvent.mouseDown(screen.getByRole('combobox', { name: 'Целевая платформа' }));
  fireEvent.click(await screen.findByText('Linux x64'));
  const input = document.querySelector('input[type="file"]') as HTMLInputElement;
  fireEvent.change(input, { target: { files: [new File(['new'], 'new.deb')] } });
  await screen.findByText('new.deb · 3 B · DEB');
  fireEvent.click(screen.getByRole('button', { name: 'Загрузить файл' }));
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', expect.stringMatching(/HTTP 413.*лимит прокси/));
  expect(screen.getByText('new.deb · 3 B · DEB')).toBeDefined();
  await waitFor(() => expect((screen.getByRole('button', { name: 'Загрузить файл' }) as HTMLButtonElement).disabled).toBe(false));
  expect(screen.queryByText('Файл загружен и сохранён.')).toBeNull();
  expect(onChange).toHaveBeenCalledOnce(); expect(onBusy).toHaveBeenLastCalledWith(false);
});
