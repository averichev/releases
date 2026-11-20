import { applicationPath } from './applications';
import type { Issue, Validation } from './publication';
export interface TauriSettings {
  public_key: string | null; public_base_url: string; endpoint: string;
  configuration: unknown | null;
  published_release_exists: boolean; ready: boolean; blockers: Issue[]; validation: Validation | null; full_integrity_checked: boolean;
  packages: {id: string; version: string; filename: string; target: string; compatible: boolean; signature_present: boolean}[];
}
export const tauriPath = (id: string) => `${applicationPath(id)}/integrations/tauri`;
