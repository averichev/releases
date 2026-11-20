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

const options = { targets: ['windows-x86_64'], file_types: ['exe', 'msi'], max_file_bytes: 1024, upload_idle_seconds: 120 };
const existing = { id: 'installer', release_id: 'release', filename: 'installer.exe', target: 'windows-x86_64', file_type: 'exe', distribution_role: 'installer' as const, signature_present: false, size_bytes: 3, sha256: 'a'.repeat(64), created_at: 1 };
it('uploads a macOS Universal package using the target offered by the server', async () => {
  let sent: FormData | undefined;
  class AcceptedXhr {
    upload = {}; status = 201; responseText = JSON.stringify({id: 'universal', target: 'darwin-universal', file_type: 'dmg'});
    onload?: () => void;
    open() {} setRequestHeader() {} send(data: FormData) { sent = data; this.onload?.(); }
  }
  vi.stubGlobal('XMLHttpRequest', AcceptedXhr);
  render(<ArtifactUploader applicationId="app" releaseId="release" artifacts={[]} options={{targets: ['darwin-universal'], file_types: ['dmg'], max_file_bytes: 1024, upload_idle_seconds: 120}} onChange={vi.fn()} onBusy={vi.fn()} />);
  fireEvent.mouseDown(screen.getByRole('combobox', {name: 'Целевая платформа'}));
  fireEvent.click(await screen.findByText('macOS Universal (Intel + Apple Silicon)'));
  fireEvent.change(document.querySelector('input[type="file"]')!, {target: {files: [new File(['new'], 'product.dmg')]}});
  await screen.findByText('product.dmg · 3 B · DMG');
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  await screen.findByText('Файл загружен и сохранён.');
  expect(sent?.get('target')).toBe('darwin-universal');
  expect((sent?.get('file') as File).name).toBe('product.dmg');
});
async function chooseFile() {
  fireEvent.mouseDown(screen.getByRole('combobox', { name: 'Целевая платформа' }));
  fireEvent.click(await screen.findByText('Windows x64'));
  fireEvent.change(document.querySelector('input[type="file"]')!, { target: { files: [new File(['new'], 'updater.exe')] } });
  await screen.findByText('updater.exe · 3 B · EXE');
}
async function chooseUpdater() {
  fireEvent.mouseDown(screen.getByRole('combobox', { name: 'Назначение' }));
  fireEvent.click(await screen.findByText('Пакет обновления (Tauri Updater)'));
}
it('sends role and signature with the initial upload even when installer has the same format', async () => {
  let sent: FormData | undefined;
  class AcceptedXhr {
    upload = {}; status = 201; responseText = JSON.stringify({ ...existing, id: 'updater', distribution_role: 'updater' });
    onload?: () => void;
    open() {} setRequestHeader() {} send(data: FormData) { sent = data; this.onload?.(); }
  }
  vi.stubGlobal('XMLHttpRequest', AcceptedXhr);
  const onChange = vi.fn();
  render(<ArtifactUploader applicationId="app" releaseId="release" artifacts={[existing]} options={options} published onChange={onChange} onBusy={vi.fn()} />);
  await chooseFile(); await chooseUpdater();
  fireEvent.click(screen.getByRole('button', { name: 'Загрузить файл' }));
  expect(await screen.findByText(/прикрепите файл .sig в этой форме/)).toBeDefined();
  expect(sent).toBeUndefined();
  fireEvent.change(screen.getByLabelText('Подпись Tauri (.sig)'), { target: { files: [new File(['signature'], 'updater.exe.sig')] } });
  fireEvent.click(screen.getByRole('button', { name: 'Загрузить файл' }));
  expect(await screen.findByText('Файл загружен и сохранён.')).toBeDefined();
  expect(sent?.get('distribution_role')).toBe('updater');
  expect((sent?.get('signature') as File).name).toBe('updater.exe.sig');
  expect((sent?.get('file') as File).name).toBe('updater.exe');
  expect(onChange).toHaveBeenCalledOnce();
});
it('checks conflicts by role and allows only one updater per target across formats', async () => {
  render(<ArtifactUploader applicationId="app" releaseId="release" artifacts={[{ ...existing, file_type: 'msi', distribution_role: 'updater' }]} options={options} onChange={vi.fn()} onBusy={vi.fn()} />);
  await chooseFile(); await chooseUpdater();
  fireEvent.click(screen.getByRole('button', { name: 'Загрузить файл' }));
  expect(await screen.findByText(/Для платформы допустим только один пакет обновления/)).toBeDefined();
});
it.each(['artifact_exists', 'updater_signature_required'])('explains %s returned by the server in Russian', async (code) => {
  class RejectedXhr {
    upload = {}; status = code === 'updater_signature_required' ? 422 : 409; responseText = JSON.stringify({ error: { code } });
    onload?: () => void;
    open() {} setRequestHeader() {} send() { this.onload?.(); }
  }
  vi.stubGlobal('XMLHttpRequest', RejectedXhr);
  render(<ArtifactUploader applicationId="app" releaseId="release" artifacts={[]} options={options} onChange={vi.fn()} onBusy={vi.fn()} />);
  await chooseFile();
  fireEvent.click(screen.getByRole('button', { name: 'Загрузить файл' }));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toMatch(code === 'artifact_exists' ? /назначением уже есть/ : /корректный файл .sig/);
});
