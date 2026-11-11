export interface User {
  id: string; username: string; role: 'admin' | 'operator'; status: 'invited' | 'active' | 'disabled';
  created_at: number; updated_at: number; last_login_at: number | null;
}
export interface Session { user: User; csrf_token: string }
export interface IssuedLink { activation_url: string; expires_at: number }
let csrfToken: string | undefined;
export function setCsrf(value?: string) { csrfToken = value; }
const messages: Record<string, string> = {
  invalid_request: 'Проверьте введённые данные. Логин: 3–64 символа a-z, 0-9, ._-. Пароль: 12–128 символов.',
  invalid_credentials: 'Неверный логин или пароль.', unauthorized: 'Сессия истекла. Войдите снова.',
  forbidden: 'Недостаточно прав или проверка безопасности не пройдена. Обновите страницу.',
  rate_limited: 'Слишком много попыток. Повторите вход через пять минут.',
  user_exists: 'Этот логин уже занят.', last_active_admin: 'Нельзя отключить или понизить последнего активного администратора.',
  invalid_token: 'Ссылка недействительна, уже использована или срок её действия истёк.',
  not_found: 'Пользователь не найден.',
  invalid_application: 'Проверьте данные приложения: название до 120 символов, описание до 2000, slug до 64 символов a-z, 0-9 и дефисов. Служебные адреса запрещены.',
  slug_exists: 'Этот slug уже занят, в том числе он может принадлежать архивному приложению.',
  invalid_icon: 'Выберите PNG, JPEG или WebP до 2 МиБ, не более 1024×1024 пикселей.',
};
export class ApiError extends Error {
  constructor(public status: number, public code: string) { super(messages[code] ?? 'Не удалось выполнить запрос. Повторите позже.'); }
}
export async function api<T>(path: string, method = 'GET', data?: unknown, signal?: AbortSignal): Promise<T> {
  const headers: Record<string, string> = { Accept: 'application/json' };
  if (method !== 'GET') {
    headers['Content-Type'] = data instanceof Blob ? data.type || 'application/octet-stream' : 'application/json'; headers['X-Veruna-Request'] = '1';
    if (csrfToken) headers['X-CSRF-Token'] = csrfToken;
  }
  const requestCsrf = csrfToken;
  let response: Response;
  try {
    response = await fetch(`/api/v1${path}`, { method, headers, credentials: 'same-origin', cache: 'no-store',
      body: data === undefined ? undefined : data instanceof Blob ? data : JSON.stringify(data),
      signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(15000)]) : AbortSignal.timeout(15000) });
  } catch (error) {
    if (signal?.aborted) throw error;
    throw new Error('Сервер недоступен. Проверьте соединение и повторите запрос.');
  }
  if (!response.ok) {
    const error: unknown = await response.json().catch(() => null);
    const code = typeof error === 'object' && error !== null && 'error' in error &&
      typeof error.error === 'object' && error.error !== null && 'code' in error.error && typeof error.error.code === 'string'
      ? error.error.code : 'internal_error';
    if (response.status === 401 && code !== 'invalid_credentials' && path !== '/auth/me' && requestCsrf === csrfToken) window.dispatchEvent(new Event('session-expired'));
    throw new ApiError(response.status, code);
  }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}
export const errorMessage = (error: unknown) => error instanceof Error ? error.message : 'Не удалось выполнить действие.';
