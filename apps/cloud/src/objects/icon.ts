import { ApiError } from '@powermove/registry/wire';
import { sha256Hex } from '@powermove/registry/git';

export async function storeIcon(bucket: R2Bucket, base64?: string): Promise<string | undefined> {
  if (base64 === undefined) return undefined;
  let bytes: Uint8Array;
  try {
    bytes = Uint8Array.from(atob(base64), char => char.charCodeAt(0));
  } catch {
    throw new ApiError({error: 'bad_request', detail: 'invalid PNG'});
  }
  if (bytes.length > 256 * 1024 || bytes.length < 8 || ![137, 80, 78, 71, 13, 10, 26, 10].every((value, index) => bytes[index] === value))
    throw new ApiError({error: 'bad_request', detail: 'invalid PNG'});
  const key = `${await sha256Hex(bytes)}.png`;
  await bucket.put(`icons/${key}`, bytes, {onlyIf: {etagDoesNotMatch: '*'}, httpMetadata: {contentType: 'image/png'}});
  return key;
}
