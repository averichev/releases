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
  invalid_release: 'Проверьте SemVer (например, 1.2.0 или 2.0.0-beta.1), название до 120 и описание до 20 000 символов.',
  release_version_exists: 'Релиз с этой версией уже существует. Build metadata после + не создаёт отдельную версию.',
  release_locked: 'Нужен черновик релиза. Создание релиза доступно только для активного приложения.',
  publication_conflict: 'Канал или релиз изменён другим администратором. Обновите данные и проверьте действие снова.',
  publication_blocked: 'Публикация запрещена: исправьте ошибки проверки готовности.',
  ambiguous_artifact: 'Подходит несколько файлов. Укажите платформу и формат или настройте дистрибутив по умолчанию.',
  updater_unavailable: 'Обновление недоступно: проверьте пакет и подпись в панели администратора.',
  invalid_updater: 'Нужен совместимый updater-пакет и корректный файл .sig от Tauri. Приватные ключи не принимаются.',
  invalid_artifact: 'Проверьте имя файла, платформу и формат. Для файла без расширения укажите формат явно.',
  artifact_exists: 'Файл с этой платформой, форматом и назначением уже есть. Для платформы допустим только один пакет обновления.',
  updater_signature_required: 'Для добавления пакета обновления в опубликованный релиз прикрепите корректный файл .sig в этой же форме.',
  default_installer_required: 'Добавление файла сделает выбор /latest (в том числе с фильтрами) неоднозначным. Сначала настройте установщик по умолчанию в разделе распространения приложения.',
  payload_too_large: 'Загрузка отклонена: превышен лимит размера файла или запроса (HTTP 413). Если файл меньше указанного лимита, администратору сервера нужно проверить лимит прокси (Nginx).',
  upload_timeout: 'Передача прервалась из-за длительного отсутствия данных. Повторите загрузку.',
  storage_full: 'На сервере закончилось место. Обратитесь к администратору сервера.',
  storage_unavailable: 'Файл в хранилище отсутствует или повреждён. Требуется проверка сервера.',
  storage_busy: 'Сервер обрабатывает другие файлы. Повторите позже.',
  invalid_icon: 'Выберите PNG, JPEG или WebP до 2 МиБ, не более 1024×1024 пикселей.',
};
const httpMessages: Partial<Record<number, string>> = {
  401: messages.unauthorized,
  403: messages.forbidden,
  404: 'Запрошенный ресурс не найден (HTTP 404). Обновите страницу.',
  408: 'Сервер прервал запрос из-за длительного ожидания данных (HTTP 408). Повторите запрос.',
  413: messages.payload_too_large,
  500: 'Внутренняя ошибка сервера (HTTP 500). Обратитесь к администратору сервера.',
  502: 'Прокси не получил корректный ответ сервера (HTTP 502). Обратитесь к администратору сервера.',
  503: 'Сервис временно недоступен (HTTP 503). Повторите запрос позже.',
  504: 'Прокси не дождался ответа сервера (HTTP 504). Обновите данные перед повторным действием.',
  507: messages.storage_full,
};
export class ApiError extends Error {
  constructor(public status: number, public code: string, public details?: unknown) { super(messages[code] ?? httpMessages[status] ?? `Не удалось выполнить запрос (HTTP ${status}). Обратитесь к администратору сервера.`); }
}
export async function api<T>(path: string, method = 'GET', data?: unknown, signal?: AbortSignal, timeoutMs = 15000): Promise<T> {
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
      signal: timeoutMs ? (signal ? AbortSignal.any([signal, AbortSignal.timeout(timeoutMs)]) : AbortSignal.timeout(timeoutMs)) : signal });
  } catch (error) {
    if (signal?.aborted) throw error;
    throw new Error('Сервер недоступен. Проверьте соединение и повторите запрос.');
  }
  if (!response.ok) {
    const error: unknown = await response.json().catch(() => null);
    throw responseError(response.status, error, path, requestCsrf);
  }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}
export const errorMessage = (error: unknown) => error instanceof Error ? error.message : 'Не удалось выполнить действие.';

function responseError(status: number, payload: unknown, path: string, requestCsrf?: string) {
  const code = typeof payload === 'object' && payload !== null && 'error' in payload &&
    typeof payload.error === 'object' && payload.error !== null && 'code' in payload.error && typeof payload.error.code === 'string'
    ? payload.error.code : 'http_error';
  if (status === 401 && code !== 'invalid_credentials' && path !== '/auth/me' && requestCsrf === csrfToken) window.dispatchEvent(new Event('session-expired'));
  return new ApiError(status, code, payload);
}
/** XHR sends the File directly; progress comes from browser transport events. */
export function upload<T>(path: string, data: FormData, signal: AbortSignal, onProgress: (percent: number) => void): Promise<T> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest(); const requestCsrf = csrfToken;
    const abort = () => xhr.abort();
    const cleanup = () => signal.removeEventListener('abort', abort);
    xhr.open('POST', `/api/v1${path}`); xhr.withCredentials = true;
    xhr.setRequestHeader('Accept', 'application/json'); xhr.setRequestHeader('X-Veruna-Request', '1');
    if (requestCsrf) xhr.setRequestHeader('X-CSRF-Token', requestCsrf);
    xhr.upload.onprogress = (event) => { if (event.lengthComputable) onProgress(Math.min(100, event.loaded / event.total * 100)); };
    xhr.onload = () => {
      cleanup(); let payload: unknown;
      try { payload = JSON.parse(xhr.responseText); } catch { payload = null; }
      if (xhr.status >= 200 && xhr.status < 300) resolve(payload as T);
      else reject(responseError(xhr.status, payload, path, requestCsrf));
    };
    xhr.onerror = () => { cleanup(); reject(new Error('Соединение прервано. Обновите список файлов перед повторной загрузкой.')); };
    xhr.onabort = () => { cleanup(); reject(new DOMException('Загрузка отменена', 'AbortError')); };
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) { cleanup(); reject(new DOMException('Загрузка отменена', 'AbortError')); return; }
    xhr.send(data);
  });
}
