import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import ArtifactUploader from './ArtifactUploader';
import type { Artifact } from '../api/releases';

const options = { targets: ['windows-x86_64', 'linux-x86_64', 'linux-aarch64', 'darwin-aarch64', 'darwin-universal', 'any'], file_types: ['exe', 'bin', 'x86_64', 'tar.gz'], max_file_bytes: 1024, upload_idle_seconds: 120 };
const existing = { download_count: 0, id: 'installer', release_id: 'release', filename: 'installer.exe', target: 'windows-x86_64', file_type: 'exe', distribution_role: 'installer' as const, signature_present: false, size_bytes: 3, sha256: 'a'.repeat(64), created_at: 1 };
async function select(name: string, label: string) {
  fireEvent.mouseDown(screen.getByRole('combobox', {name}));
  fireEvent.click(await screen.findByText(label, {selector: '.ant-select-item-option-content'}));
}
async function choose(kind: string, os = 'Windows', target = os === 'Linux' ? 'linux-x86_64' : 'windows-x86_64') {
  await select('Тип артефакта', kind);
  await select('Операционная система', os);
  if (os !== 'Любая ОС') await select('Архитектура', target);
}
function chooseFile(filename: string) {
  fireEvent.change(document.querySelector('input[type="file"]')!, {target: {files: [new File(['new'], filename)]}});
}
function acceptedUpload() {
  let sent: FormData | undefined;
  class AcceptedXhr {
    upload = {}; status = 201; responseText = '{}'; onload?: () => void;
    open() {} setRequestHeader() {} send(data: FormData) { sent = data; this.onload?.(); }
  }
  vi.stubGlobal('XMLHttpRequest', AcceptedXhr);
  return () => sent;
}
function renderUploader(artifacts: Artifact[] = [existing], published = false) {
  const onChange = vi.fn(); const onBusy = vi.fn();
  render(<ArtifactUploader applicationId="app" releaseId="release" artifacts={artifacts} options={options} published={published} onChange={onChange} onBusy={onBusy} />);
  return {onChange, onBusy};
}

it('reveals OS, architecture and files in order without assuming a platform', async () => {
  renderUploader([]);
  expect(screen.queryByRole('combobox', {name: 'Операционная система'})).toBeNull();
  expect(document.querySelector('input[type="file"]')).toBeNull();
  await select('Тип артефакта', 'Установочный файл');
  expect(screen.queryByRole('combobox', {name: 'Архитектура'})).toBeNull();
  await select('Операционная система', 'Linux');
  expect(document.querySelector('input[type="file"]')).toBeNull();
  fireEvent.mouseDown(screen.getByRole('combobox', {name: 'Архитектура'}));
  expect(screen.queryByText('darwin-universal', {selector: '.ant-select-item-option-content'})).toBeNull();
  expect(screen.queryByText('windows-x86_64', {selector: '.ant-select-item-option-content'})).toBeNull();
  expect(await screen.findByText('linux-aarch64', {selector: '.ant-select-item-option-content'})).toBeDefined();
  fireEvent.click(await screen.findByText('linux-x86_64', {selector: '.ant-select-item-option-content'}));
  expect(screen.getByText('linux-x86_64', {selector: 'code'})).toBeDefined();
  expect(screen.getByRole('heading', {name: '4. Один файл'})).toBeDefined();
  expect(document.querySelectorAll('input[type="file"]')).toHaveLength(1);
});

it('explains a proxy upload limit and keeps the selected file for retry', async () => {
  class RejectedXhr {
    upload = {}; status = 413; responseText = '<html>413 Request Entity Too Large</html>'; onload?: () => void;
    open() {} setRequestHeader() {} send() { this.onload?.(); }
  }
  vi.stubGlobal('XMLHttpRequest', RejectedXhr);
  const {onChange, onBusy} = renderUploader([]);
  await choose('Установочный файл', 'Linux'); chooseFile('new.deb');
  await screen.findByText('new.deb · 3 B · DEB');
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  expect((await screen.findByRole('alert')).textContent).toMatch(/HTTP 413.*лимит прокси/);
  expect(screen.getByText('new.deb · 3 B · DEB')).toBeDefined();
  await waitFor(() => expect((screen.getByRole('button', {name: 'Загрузить файл'}) as HTMLButtonElement).disabled).toBe(false));
  expect(screen.queryByText('Файл загружен и сохранён.')).toBeNull();
  expect(onChange).toHaveBeenCalledOnce(); expect(onBusy).toHaveBeenLastCalledWith(false);
});

it.each(['darwin-aarch64', 'darwin-universal'])('shows the canonical macOS build identifier %s and sends it unchanged', async (target) => {
  const sent = acceptedUpload(); renderUploader([]);
  await choose('Установочный файл', 'macOS (darwin)', target); chooseFile(`product-1.0.0-${target}.dmg`);
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  await screen.findByText('Файл загружен и сохранён.');
  expect(sent()?.get('target')).toBe(target);
});

it('requires the Tauri signature in the same upload even on a draft', async () => {
  const sent = acceptedUpload(); const {onChange} = renderUploader();
  await choose('Пакет обновления Tauri'); chooseFile('updater.exe');
  expect(document.querySelectorAll('input[type="file"]')).toHaveLength(2);
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  expect((await screen.findByRole('alert')).textContent).toMatch(/Прикрепите файл .sig/);
  expect(sent()).toBeUndefined();
  fireEvent.change(screen.getByLabelText('Подпись Tauri (.sig)'), {target: {files: [new File(['signature'], 'updater.exe.sig')]}});
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  await screen.findByText('Файл загружен и сохранён.');
  expect(sent()?.get('distribution_role')).toBe('updater');
  expect((sent()?.get('signature') as File).name).toBe('updater.exe.sig');
  expect(sent()?.has('standalone_signature')).toBe(false);
  expect(onChange).toHaveBeenCalledOnce();
});

it('accepts AppImage for Linux Tauri and explains why DEB cannot be used', async () => {
  acceptedUpload(); renderUploader([]);
  await choose('Пакет обновления Tauri', 'Linux'); chooseFile('setup.deb');
  expect((await screen.findByRole('alert')).textContent).toMatch(/AppImage/);
  expect((screen.getByRole('button', {name: 'Загрузить файл'}) as HTMLButtonElement).disabled).toBe(true);
  chooseFile('update.AppImage');
  expect(await screen.findByText('update.AppImage · 3 B · APPIMAGE')).toBeDefined();
});

it('checks Tauri uniqueness across file formats', async () => {
  renderUploader([{...existing, file_type: 'msi', distribution_role: 'updater'}]);
  await choose('Пакет обновления Tauri'); chooseFile('updater.exe');
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  expect((await screen.findByRole('alert')).textContent).toMatch(/только один пакет обновления Tauri/);
});

it.each(['artifact_exists', 'updater_signature_required', 'invalid_standalone'])('explains server error %s in Russian', async (code) => {
  class RejectedXhr {
    upload = {}; status = code === 'artifact_exists' ? 409 : 422; responseText = JSON.stringify({error: {code}}); onload?: () => void;
    open() {} setRequestHeader() {} send() { this.onload?.(); }
  }
  vi.stubGlobal('XMLHttpRequest', RejectedXhr); renderUploader([]);
  await choose('Установочный файл'); chooseFile('setup.exe');
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toMatch(code === 'artifact_exists' ? /типом уже есть/ : code === 'invalid_standalone' ? /подпись .jws/ : /корректный файл .sig/);
});

it.each([
  ['Исполняемый файл', 'executable', 'program.exe', 'Windows', 'windows-x86_64'],
  ['Исполняемый файл', 'executable', 'program-0.7.0-linux-x86_64', 'Linux', 'linux-x86_64'],
  ['Исполняемый файл', 'executable', 'program.x86_64', 'Linux', 'linux-x86_64'],
  ['Установочный файл', 'installer', 'setup.exe', 'Windows', 'windows-x86_64'],
  ['Архив', 'archive', 'program.tar.gz', 'Любая ОС', 'any'],
])('uploads %s %s %s as one file', async (kind, role, filename, os, target) => {
  const sent = acceptedUpload(); renderUploader([], true);
  await choose(kind, os, target); chooseFile(filename);
  expect(document.querySelectorAll('input[type="file"]')).toHaveLength(1);
  if (filename === 'program-0.7.0-linux-x86_64') expect(await screen.findByText(`${filename} · 3 B · BIN`)).toBeDefined();
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  await screen.findByText('Файл загружен и сохранён.');
  expect(sent()?.get('distribution_role')).toBe(role); expect(sent()?.get('target')).toBe(target);
  expect((sent()?.get('file') as File).name).toBe(filename);
  expect(sent()?.has('signature')).toBe(false); expect(sent()?.has('standalone_signature')).toBe(false); expect(sent()?.has('file_type')).toBe(false);
});

it('requires standalone JWS and resets architecture and files when OS changes', async () => {
  const sent = acceptedUpload(); renderUploader([], true);
  await choose('Пакет обновления', 'Linux'); chooseFile('program-0.7.0-linux-x86_64');
  fireEvent.change(screen.getByLabelText('Подпись издателя (.jws)'), {target: {files: [new File(['old-signature'], 'old.jws')]}});
  await select('Операционная система', 'Windows');
  expect(document.querySelector('input[type="file"]')).toBeNull();
  await select('Архитектура', 'windows-x86_64');
  expect(screen.queryByText('old.jws')).toBeNull();
  chooseFile('program.exe');
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  expect(await screen.findByText(/Прикрепите файл .jws/)).toBeDefined();
  expect(sent()).toBeUndefined();
  fireEvent.change(screen.getByLabelText('Подпись издателя (.jws)'), {target: {files: [new File(['signed-metadata'], 'program.jws')]}});
  fireEvent.click(screen.getByRole('button', {name: 'Загрузить файл'}));
  await screen.findByText('Файл загружен и сохранён.');
  expect(sent()?.get('target')).toBe('windows-x86_64'); expect(sent()?.get('distribution_role')).toBe('executable');
  expect((sent()?.get('standalone_signature') as File).name).toBe('program.jws'); expect(sent()?.has('signature')).toBe(false);
});

it('clears files when the workflow changes and rejects invalid standalone signature files', async () => {
  renderUploader([]);
  await choose('Пакет обновления', 'Linux'); chooseFile('program');
  for (const file of [new File([], 'empty.jws'), new File(['a'.repeat(4097)], 'large.jws'), new File(['signature'], 'wrong.sig')]) {
    fireEvent.change(screen.getByLabelText('Подпись издателя (.jws)'), {target: {files: [file]}});
    expect((await screen.findByRole('alert')).textContent).toMatch(/непустой файл .jws до 4 KiB/);
  }
  await select('Тип артефакта', 'Архив');
  expect(document.querySelector('input[type="file"]')).toBeNull();
  expect(screen.queryByRole('combobox', {name: 'Архитектура'})).toBeNull();
});
