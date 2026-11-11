import { api, ApiError } from './client';
export interface Application {
  id: string; slug: string; name: string; description: string; icon_url: string | null;
  status: 'active' | 'archived'; visibility: 'private' | 'public'; created_at: number; updated_at: number;
}
export interface ApplicationPage { items: Application[]; total: number; page: number; per_page: number }
export interface ApplicationValues { name: string; slug: string; description: string; visibility: 'private' | 'public' }
export const applicationPath = (id: string) => `/admin/applications/${encodeURIComponent(id)}`;
export const date = (value: number) => new Date(value * 1000).toLocaleString('ru-RU');
export async function saveIcon(id: string, file: File) { return api<Application>(`${applicationPath(id)}/icon`, 'POST', file); }
export function applicationError(error: unknown) {
  return error instanceof ApiError && error.code === 'not_found' ? 'Приложение не найдено.' : error instanceof Error ? error.message : 'Не удалось выполнить действие.';
}
export const reservedSlugs = ['api', 'admin', 'assets', 'auth', 'login', 'logout', 'static', 'health', 'activate', 'favicon', 'index'];
export const validSlug = (value: string) => /^(?:[a-z0-9]|[a-z0-9][a-z0-9-]{0,62}[a-z0-9])$/.test(value) && !reservedSlugs.includes(value);
const transliteration: Record<string, string> = {
  а: 'a', б: 'b', в: 'v', г: 'g', д: 'd', е: 'e', ё: 'yo', ж: 'zh', з: 'z', и: 'i', й: 'y', к: 'k', л: 'l', м: 'm', н: 'n', о: 'o', п: 'p', р: 'r', с: 's', т: 't', у: 'u', ф: 'f', х: 'kh', ц: 'ts', ч: 'ch', ш: 'sh', щ: 'shch', ъ: '', ы: 'y', ь: '', э: 'e', ю: 'yu', я: 'ya',
};
export function suggestSlug(name: string) {
  return [...name.toLowerCase()].map((c) => transliteration[c] ?? c).join('').normalize('NFKD').replace(/[\u0300-\u036f]/g, '')
    .replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64).replace(/-+$/g, '');
}
