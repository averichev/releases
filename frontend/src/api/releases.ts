import { ApiError } from './client';
import { applicationPath } from './applications';
export interface Release {
  id: string; application_id: string; version: string; title: string | null; release_notes: string;
  status: 'draft'; created_at: number; updated_at: number; published_at: number | null; artifact_count: number; total_size_bytes: number;
}
export interface ReleaseValues { version: string; title?: string; release_notes: string }
export interface ReleasePage { items: Release[]; total: number; page: number; per_page: number }
export interface Artifact {
  id: string; release_id: string; filename: string; target: string; file_type: string; size_bytes: number; sha256: string; created_at: number;
}
export interface UploadOptions { targets: string[]; file_types: string[]; max_file_bytes: number; upload_idle_seconds: number }
export const releasesPath = (app: string) => `${applicationPath(app)}/releases`;
export const releasePath = (app: string, release: string) => `${releasesPath(app)}/${encodeURIComponent(release)}`;
export const releaseRoute = (app: string, release: string) => `/admin/applications/${encodeURIComponent(app)}/releases/${encodeURIComponent(release)}`;
export const artifactsPath = (app: string, release: string) => `${releasePath(app, release)}/artifacts`;
export function releaseError(error: unknown) { return error instanceof ApiError && error.code === 'not_found' ? 'Приложение, релиз или файл не найден.' : error instanceof Error ? error.message : 'Не удалось выполнить действие.'; }
export function size(bytes: number) {
  if (bytes === 0) return '0 B';
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 3);
  return `${(bytes / 1024 ** unit).toLocaleString('ru-RU', { maximumFractionDigits: unit ? 2 : 0 })} ${['B', 'KiB', 'MiB', 'GiB'][unit]}`;
}
export const targetName = (value: string) => ({ 'windows-x86_64': 'Windows x64', 'windows-aarch64': 'Windows ARM64', 'linux-x86_64': 'Linux x64', 'linux-aarch64': 'Linux ARM64', 'darwin-x86_64': 'macOS x64', 'darwin-aarch64': 'macOS ARM64', any: 'Любая платформа' })[value] ?? value;
// Backend remains authoritative, including u64 bounds of numeric components.
export const validVersion = (value: string) => /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(value.trim()) && value.trim().length <= 201;
export function detectedType(filename: string) {
  const lower = filename.toLowerCase();
  return ['tar.gz', 'tar.xz', 'tar.bz2', 'tar.zst'].find((ext) => lower.endsWith(`.${ext}`)) ?? (lower.lastIndexOf('.') > 0 ? lower.split('.').pop() : undefined);
}
