#!/bin/bash
# The versioned files go first. A single pointer exposes the appcast and policy.
set -euo pipefail
[[ "${PM_NATIVE_PUBLISH:-}" == true && "${GITHUB_EVENT_NAME:-}" == workflow_dispatch ]]
version="${PM_NATIVE_VERSION:?}"
[[ "$version" =~ ^[1-9][0-9]*[.][0-9]+[.][0-9]+$ ]]
[[ "${GITHUB_REF:-}" == "refs/tags/native-v$version" ]]
[[ "${AWS_DEFAULT_REGION:-}" == auto ]]
: "${AWS_ACCESS_KEY_ID:?}" "${AWS_SECRET_ACCESS_KEY:?}"
stage="${PM_NATIVE_STAGE:?}"
bucket=powermove-tars
endpoint=https://98dbb6e46f4901896327e5ff4035d637.r2.cloudflarestorage.com
prefix="native/$version"
asset="Powermove-Native-$version-mac-arm64"
[[ "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["mode"])' "$stage/evidence/verification.json")" == publish ]]
for file in "$asset.dmg" "$asset.zip" appcast.xml policy.json; do
  test -s "$stage/$file"
done

put_versioned() {
  local file="$1" mime="$2" key="$prefix/$1" prior sha size
  sha="$(shasum -a 256 "$stage/$file" | cut -d ' ' -f 1)"
  size="$(wc -c < "$stage/$file" | tr -d '[:space:]')"
  prior="$(aws --endpoint-url "$endpoint" s3api list-objects-v2 --bucket "$bucket" \
    --prefix "$key" --query 'Contents[].Key' --output text)"
  if [[ "$prior" == "$key" ]]; then
    echo "Checking existing $file"
  else
    [[ -z "$prior" || "$prior" == None ]]
    aws --endpoint-url "$endpoint" s3 cp "$stage/$file" "s3://$bucket/$key" \
      --content-type "$mime" --metadata "sha256=$sha" --only-show-errors
  fi
  [[ "$(aws --endpoint-url "$endpoint" s3api head-object --bucket "$bucket" --key "$key" \
    --query 'Metadata.sha256' --output text)" == "$sha" ]]
  [[ "$(aws --endpoint-url "$endpoint" s3api head-object --bucket "$bucket" --key "$key" \
    --query 'ContentLength' --output text)" == "$size" ]]
}

put_versioned "$asset.dmg" application/x-apple-diskimage
put_versioned "$asset.zip" application/zip
put_versioned appcast.xml application/rss+xml
put_versioned policy.json application/json
for file in "$asset.dmg" "$asset.zip"; do
  status="$(curl --silent --show-error --head --output /dev/null --write-out '%{http_code}' \
    "https://cloud.trypowermove.com/$prefix/$file")"
  test "$status" = 200
done
printf '%s\n' "$version" > "$stage/version"
aws --endpoint-url "$endpoint" s3 cp "$stage/version" "s3://$bucket/native/latest/version" \
  --content-type text/plain --only-show-errors
