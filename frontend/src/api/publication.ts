import { ApiError } from './client';
export interface Issue { code: string; artifact_id: string | null; message: string }
export interface Validation { state: 'ready' | 'warnings' | 'errors'; warnings: Issue[]; errors: Issue[] }
export interface PublicationCheck { release_revision: number; validation: Validation }
export interface PublicationEvent { id: number; version: string; username: string; created_at: number }
export interface TauriResponse { version: string; pub_date: string; url: string; signature: string; notes: string }
export interface TauriPreview { state: string; reason: string; message: string; selected_version: string | null; artifact_id: string | null; artifact_filename?: string | null; signature_present: boolean; public_downloads_available: boolean; response: TauriResponse | null }
export function serverValidation(error: unknown): Validation | undefined {
  if (error instanceof ApiError && typeof error.details === 'object' && error.details !== null && 'validation' in error.details) return error.details.validation as Validation;
}
