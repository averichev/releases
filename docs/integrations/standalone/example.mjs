// Offline protocol example: signs/verifies files; never installs or executes them.
import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';

const typ = 'releases-standalone+jws';
const keyId = jwk => createHash('sha256')
  .update(JSON.stringify({ crv: 'Ed25519', kty: 'OKP', x: jwk.x }))
  .digest('base64url'); // RFC 7638; required public JWK members in lexical order.
const fields = ['schema_version', 'application_id', 'version', 'os', 'arch', 'size_bytes', 'sha256'].sort();
async function metadata(product_id, version, target, file) {
  assert(/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/.test(product_id), 'noncanonical product UUID');
  const parts = target.split('-');
  assert(parts.length === 2 && ['windows', 'linux', 'darwin'].includes(parts[0]) && ['x86_64', 'aarch64'].includes(parts[1]), 'unsupported target');
  const hash = createHash('sha256');
  let size_bytes = 0;
  for await (const chunk of createReadStream(file)) { hash.update(chunk); size_bytes += chunk.length; }
  assert(size_bytes > 0 && Number.isSafeInteger(size_bytes), 'invalid file size');
  return { schema_version: 1, application_id: product_id, version, os: parts[0], arch: parts[1], size_bytes, sha256: hash.digest('hex') };
}
const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
function decode(value) {
  const bytes = Buffer.from(value, 'base64url');
  assert(bytes.toString('base64url') === value, 'noncanonical base64url');
  return bytes;
}
const [command, ...args] = process.argv.slice(2);
if (command === 'keygen' && args.length === 1) {
  const { privateKey, publicKey } = generateKeyPairSync('ed25519');
  await writeFile(`${args[0]}.pem`, privateKey.export({ type: 'pkcs8', format: 'pem' }), { flag: 'wx', mode: 0o600 });
  const jwk = publicKey.export({ format: 'jwk' });
  await writeFile(`${args[0]}.public.jwk`, `${JSON.stringify(jwk)}\n`, { flag: 'wx' });
  console.log(JSON.stringify({ public_key: jwk.x })); // Only this public value goes to Releases.
} else if (command === 'sign' && (args.length === 5 || args.length === 6 && args[5] === '--kid')) {
  const [keyFile, product, version, target, file] = args;
  const key = createPrivateKey(await readFile(keyFile));
  assert(key.asymmetricKeyType === 'ed25519');
  const header = { alg: 'EdDSA', typ };
  if (args[5] === '--kid') header.kid = keyId(createPublicKey(key).export({ format: 'jwk' }));
  const input = `${encode(header)}.${encode(await metadata(product, version, target, file))}`;
  console.log(`${input}.${sign(null, Buffer.from(input), key).toString('base64url')}`);
} else if (command === 'verify' && args.length === 6) {
  const [keyFile, product, version, target, file, jwsFile] = args;
  const jwk = JSON.parse(await readFile(keyFile, 'utf8'));
  assert(jwk.kty === 'OKP' && jwk.crv === 'Ed25519' && !('d' in jwk) && decode(jwk.x).length === 32);
  const jws = (await readFile(jwsFile, 'utf8')).trim();
  assert(jws.length > 0 && jws.length <= 4096);
  const parts = jws.split('.');
  assert(parts.length === 3);
  const [header, payload, signature] = parts;
  const protectedHeader = JSON.parse(decode(header));
  const expected = { alg: 'EdDSA', typ };
  if (Object.hasOwn(protectedHeader, 'kid')) expected.kid = keyId(jwk);
  assert.deepEqual(protectedHeader, expected);
  assert(verify(null, Buffer.from(`${header}.${payload}`), createPublicKey({ key: jwk, format: 'jwk' }), decode(signature)), 'invalid signature');
  const m = JSON.parse(decode(payload));
  assert.deepEqual(Object.keys(m).sort(), fields);
  assert.deepEqual(m, await metadata(product, version, target, file), 'metadata/content mismatch');
  console.log('Verified signature, product, version, platform, size and SHA-256');
} else {
  throw new Error('Usage: keygen <prefix> | sign <private.pem> <product-uuid> <version> <target> <file> [--kid] | verify <public.jwk> <product-uuid> <version> <target> <file> <file.jws>');
}
