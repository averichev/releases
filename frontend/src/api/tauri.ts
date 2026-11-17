import { applicationPath } from './applications';
import type { Issue, Validation } from './publication';
export interface TauriSettings {
  public_key: string | null; public_base_url: string; endpoint: string; endpoints: {stable: string; beta: string};
  configuration: unknown | null; channel: string; current_version: string | null;
  published_release_exists: boolean; ready: boolean; blockers: Issue[]; validation: Validation | null; full_integrity_checked: boolean;
  packages: {id: string; filename: string; target: string; compatible: boolean; signature_present: boolean}[];
}
export const tauriPath = (id: string) => `${applicationPath(id)}/integrations/tauri`;
