import { applicationPath } from './applications';

export interface StandaloneSettings {
  application_id: string; product_id: string | null; public_key: string | null; key_id: string | null;
  product_id_locked: boolean; public_key_locked: boolean;
}
export const standalonePath = (app: string) => `${applicationPath(app)}/integrations/standalone`;
export const standaloneSettingsRoute = (app: string) => `/admin/applications/${encodeURIComponent(app)}/settings#signed-updates`;
export const validProductId = (value: string) => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value);

/** Only public Ed25519 JWKs or their x component; never send private material. */
export function standalonePublicKey(value: string): string | null {
  const text = value.trim();
  if (!text) return null;
  let key = text;
  if (text.startsWith('{')) {
    let jwk: unknown;
    try { jwk = JSON.parse(text); } catch { throw new Error('Публичный JWK содержит некорректный JSON.'); }
    if (typeof jwk !== 'object' || jwk === null || 'd' in jwk || !('kty' in jwk) || jwk.kty !== 'OKP'
      || !('crv' in jwk) || jwk.crv !== 'Ed25519' || !('x' in jwk) || typeof jwk.x !== 'string') {
      throw new Error('Нужен публичный Ed25519 JWK с полями kty=OKP, crv=Ed25519 и x. Приватный ключ не принимается.');
    }
    key = jwk.x;
  }
  if (!/^[A-Za-z0-9_-]{43}$/.test(key)) throw new Error('Вставьте публичный Ed25519 JWK или его поле x: 43 символа base64url без знака =.');
  return key;
}

export function missingStandaloneSettings(settings: StandaloneSettings): string | undefined {
  if (!settings.product_id && !settings.public_key) return 'Не заданы UUID продукта и публичный ключ.';
  if (!settings.product_id) return 'Не задан UUID продукта.';
  if (!settings.public_key) return 'Не задан публичный ключ.';
}
