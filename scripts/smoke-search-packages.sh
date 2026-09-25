#!/usr/bin/env bash
set -euo pipefail

PACKAGE_NAME=io-github-briolabs-maestria-search
if [[ $# -ne 1 || ! -d "$1" ]]; then
  echo "usage: $0 PACKAGE_DIRECTORY" >&2
  exit 2
fi

if ! command -v dpkg-deb >/dev/null || ! command -v dpkg-query >/dev/null; then
  echo "dpkg-deb and dpkg-query are required to verify the installed search package" >&2
  exit 1
fi

package_dir="$(realpath "$1")"
shopt -s nullglob
packages=("$package_dir"/*.deb)
if [[ ${#packages[@]} -ne 1 ]]; then
  echo "expected exactly one Debian package in $package_dir, found ${#packages[@]}" >&2
  exit 1
fi
package="${packages[0]}"
package_id="$(dpkg-deb --field "$package" Package)"
package_version="$(dpkg-deb --field "$package" Version)"
package_architecture="$(dpkg-deb --field "$package" Architecture)"
if [[ "$package_id" != "$PACKAGE_NAME" || "$package_architecture" != amd64 || -z "$package_version" ]]; then
  echo "unexpected Debian package metadata: package=$package_id architecture=$package_architecture version=$package_version" >&2
  exit 1
fi

installed="$(dpkg-query --show --showformat='${Status}|${Version}|${Architecture}' "$PACKAGE_NAME")"
expected_installed="install ok installed|$package_version|$package_architecture"
if [[ "$installed" != "$expected_installed" ]]; then
  echo "installed package does not match $package: got $installed" >&2
  exit 1
fi

binary="$(command -v maestria-search || true)"
if [[ "$binary" != /usr/bin/maestria-search || "$(realpath "$binary")" != /usr/bin/maestria-search ]]; then
  echo "expected installed PATH executable /usr/bin/maestria-search, got ${binary:-not found}" >&2
  exit 1
fi
binary_owner="$(dpkg-query --search /usr/bin/maestria-search)"
if [[ "$binary_owner" != "$PACKAGE_NAME: /usr/bin/maestria-search" ]]; then
  echo "installed executable is not owned by $PACKAGE_NAME: $binary_owner" >&2
  exit 1
fi

umask 077
root="$(mktemp -d "${TMPDIR:-/tmp}/maestria-search-package-smoke.XXXXXX")"
instance="$root/instance"
approved_root="$root/approved-root"
credential_file="$root/external-grant.credential"
socket_path="$instance/system/daemon.sock"
daemon_log="$root/daemon.log"
daemon_pid=

process_state() {
  local pid="$1" stat_line
  [[ -r "/proc/$pid/stat" ]] || return 0
  IFS= read -r stat_line < "/proc/$pid/stat" || return 0
  stat_line="${stat_line##*) }"
  printf '%s\n' "${stat_line%% *}"
}

process_is_live() {
  local state
  state="$(process_state "$1")"
  [[ -n "$state" && "$state" != Z && "$state" != X ]]
}

terminate_daemon() {
  [[ -n "$daemon_pid" ]] || return 0
  local forced=false
  if process_is_live "$daemon_pid"; then
    kill -TERM "$daemon_pid" 2>/dev/null || true
    for attempt in {1..200}; do
      process_is_live "$daemon_pid" || break
      sleep 0.05
    done
    if process_is_live "$daemon_pid"; then
      kill -KILL "$daemon_pid" 2>/dev/null || true
      forced=true
    fi
  fi
  wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=
  [[ "$forced" == false ]]
}

cleanup() {
  local exit_status=$?
  trap - EXIT INT TERM
  terminate_daemon || true
  rm -rf -- "$root"
  exit "$exit_status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$root/home" "$root/config" "$root/cache" "$root/data" "$root/state" "$approved_root"
export HOME="$root/home"
export XDG_CONFIG_HOME="$root/config"
export XDG_CACHE_HOME="$root/cache"
export XDG_DATA_HOME="$root/data"
export XDG_STATE_HOME="$root/state"

fail() {
  echo "search package smoke failed: $*" >&2
  exit 1
}

phrase='The cobalt kestrel carries a silver compass beneath northern rain'
fixture="$approved_root/search-fixture.md"
cat > "$fixture" <<'EOF'
# Search-only package smoke fixture
The cobalt kestrel carries a silver compass beneath northern rain.
EOF

# Source-aware common-document fixtures travel through the installed binary,
# not parser test helpers or a model provider.
python3 - "$approved_root" <<'PY'
from pathlib import Path
import sys
import zipfile

root = Path(sys.argv[1])
with zipfile.ZipFile(root / "letter.docx", "w", zipfile.ZIP_DEFLATED) as archive:
    archive.writestr(
        "[Content_Types].xml",
        '<?xml version="1.0" encoding="utf-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
    )
    archive.writestr(
        "_rels/.rels",
        '<?xml version="1.0" encoding="utf-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>',
    )
    archive.writestr(
        "word/document.xml",
        '<?xml version="1.0" encoding="utf-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>A coral compass guides the observatory archivist.</w:t></w:r></w:p></w:body></w:document>',
    )


def save_pdf(path, objects):
    output = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for number, body in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f"{number} 0 obj\n".encode() + body + b"\nendobj\n")
    startxref = len(output)
    output.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        output.extend(f"{offset:010d} 00000 n \n".encode())
    output.extend(
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{startxref}\n%%EOF\n".encode()
    )
    path.write_bytes(output)


pdf_text = b"BT /F1 15 Tf 72 720 Td (The azure narwhal guards the archive) Tj ET\n"
save_pdf(
    root / "text.pdf",
    [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        f"<< /Length {len(pdf_text)} >>\nstream\n".encode() + pdf_text + b"endstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ],
)
pixel = b"\x80\x80\x80"
image_draw = b"q 612 0 0 792 0 0 cm /Im1 Do Q\n"
save_pdf(
    root / "scan.pdf",
    [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>",
        f"<< /Length {len(image_draw)} >>\nstream\n".encode() + image_draw + b"endstream",
        b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n"
        + pixel
        + b"\nendstream",
    ],
)
PY

"$binary" init --instance-dir "$instance" --read-root "$approved_root" >"$root/init.out" 2>"$root/init.err" || {
  cat "$root/init.err" >&2
  fail "could not initialize an instance with the explicit approved root"
}
provider_realm=
while IFS='=' read -r key value; do
  if [[ "$key" == realm_id ]]; then
    provider_realm="$value"
    break
  fi
done < "$instance/manifest.txt"
if [[ ! "$provider_realm" =~ ^[0-9a-f]{64}$ ]]; then
  fail "initialized provider realm is not a 64-character lowercase hexadecimal ID"
fi

consumer_realm=
ungranted_realm=
for candidate in {1..32}; do
  candidate_realm="$(printf '%064x' "$candidate")"
  if [[ -z "$consumer_realm" && "$candidate_realm" != "$provider_realm" ]]; then
    consumer_realm="$candidate_realm"
  elif [[ -n "$consumer_realm" && -z "$ungranted_realm" && "$candidate_realm" != "$provider_realm" && "$candidate_realm" != "$consumer_realm" ]]; then
    ungranted_realm="$candidate_realm"
    break
  fi
done
if [[ -z "$consumer_realm" || -z "$ungranted_realm" ]]; then
  fail "could not choose distinct provider, granted, and ungranted realm IDs"
fi

"$binary" start --instance-dir "$instance" >"$daemon_log" 2>&1 &
daemon_pid=$!
owner_status="$root/owner-status.json"
owner_error="$root/owner-status.err"
owner_ready=false
for attempt in {1..600}; do
  if "$binary" owner roots status --instance-dir "$instance" >"$owner_status" 2>"$owner_error"; then
    if python3 - "$owner_status" "$approved_root" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    status = json.load(stream)
roots = status.get("roots")
if status.get("approved_root_count") != 1 or not isinstance(roots, list) or len(roots) != 1:
    raise SystemExit(1)
if roots[0].get("path") != sys.argv[2]:
    raise SystemExit(1)
PY
    then
      owner_ready=true
      break
    fi
  fi
  if ! process_is_live "$daemon_pid"; then
    cat "$daemon_log" >&2
    fail "read-only owner daemon exited before becoming ready"
  fi
  sleep 0.05
done
if [[ "$owner_ready" != true ]]; then
  cat "$daemon_log" >&2
  cat "$owner_error" >&2
  fail "read-only owner daemon did not expose the approved-root status"
fi
if [[ ! -S "$socket_path" ]]; then
  fail "daemon did not bind its private Unix-domain socket at $socket_path"
fi

if ! grant_output="$("$binary" owner grant create-external \
  --instance-dir "$instance" \
  --consumer-realm "$consumer_realm" \
  --credential-file "$credential_file" \
  --access search-and-open-evidence \
  --max-sensitivity internal \
  --max-results 1 \
  --max-evidence-bytes 512 \
  --expires-in-seconds 900 2>"$root/grant-create.err")"; then
  cat "$root/grant-create.err" >&2
  fail "could not create the bounded external search grant"
fi
for expected in \
  "consumer_realm=$consumer_realm" \
  'access=search-and-open-evidence' \
  'max_results=1' \
  'max_sensitivity=internal' \
  'max_evidence_bytes=512' \
  'state=active'; do
  [[ "$grant_output" == *"$expected"* ]] || fail "created grant omitted expected policy field $expected"
done
if [[ "$grant_output" != *"allowed_roots=[\"$approved_root\"]"* ]]; then
  fail "grant created without --read-root did not freeze the initially approved root"
fi
grant_digest="$(sed -n 's/^grant_token_digest=//p' <<<"$grant_output")"
if [[ ! "$grant_digest" =~ ^[0-9a-f]{64}$ || ! -s "$credential_file" ]]; then
  fail "grant creation did not produce a digest and private credential file"
fi
if [[ "$(stat -c '%a' "$credential_file")" != 600 ]]; then
  fail "external grant credential file is not private mode 0600"
fi

consumer_args=(--socket-path "$socket_path" --consumer-realm "$consumer_realm" --credential-file "$credential_file")
ungranted_args=(--socket-path "$socket_path" --consumer-realm "$ungranted_realm" --credential-file "$credential_file")
status_json="$root/consumer-status.json"
if ! "$binary" status "${consumer_args[@]}" >"$status_json" 2>"$root/consumer-status.err"; then
  cat "$root/consumer-status.err" >&2
  fail "granted consumer status request failed"
fi
python3 - "$status_json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
if response.get("type") != "status" or not isinstance(response.get("data"), dict):
    raise SystemExit("consumer status returned an unexpected response type")
PY

indexing_json="$root/consumer-indexing-status.json"
indexing_error="$root/consumer-indexing-status.err"
indexing_ready=false
for attempt in {1..1200}; do
  if "$binary" indexing-status "${consumer_args[@]}" >"$indexing_json" 2>"$indexing_error"; then
    if python3 - "$indexing_json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
data = response.get("data")
if response.get("type") != "indexing_status" or not isinstance(data, dict):
    raise SystemExit(1)
if (
    data.get("approved_root_count") == 1
    and data.get("indexed_file_count") == 3
    and data.get("ocr_needed_file_count") == 1
    and data.get("excluded_file_count") == 1
    and data.get("exclusions_by_reason", {}).get("needs_ocr") == 1
    and data.get("pending_file_count") == 0
    and data.get("scanning") is False
    and data.get("last_scan_error") is False
    and data.get("last_scan_unix_ms") is not None
):
    raise SystemExit(0)
raise SystemExit(1)
PY
    then
      indexing_ready=true
      break
    fi
  elif ! process_is_live "$daemon_pid"; then
    cat "$daemon_log" >&2
    fail "read-only owner daemon exited before indexing completed"
  fi
  sleep 0.05
done
if [[ "$indexing_ready" != true ]]; then
  cat "$indexing_json" >&2 || true
  cat "$indexing_error" >&2 || true
  fail "approved Markdown, DOCX, PDF and OCR-needed status did not settle within the bounded wait"
fi

search_json="$root/consumer-search.json"
if ! "$binary" search "${consumer_args[@]}" --limit 100 "$phrase" >"$search_json" 2>"$root/consumer-search.err"; then
  cat "$root/consumer-search.err" >&2
  fail "granted exact-phrase search request failed"
fi
evidence_id="$(python3 - "$search_json" "$phrase" "$fixture" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
if response.get("type") != "search" or not isinstance(response.get("data"), dict):
    raise SystemExit("search returned an unexpected response type")
data = response["data"]
if data.get("query") != sys.argv[2]:
    raise SystemExit("search response did not preserve the exact query")
evidence = data.get("evidence")
if not isinstance(evidence, list) or not evidence or len(evidence) > 1:
    raise SystemExit("bounded grant did not return exactly one or fewer search results")
fixture_name = sys.argv[3].rsplit("/", 1)[-1]
match = next((item for item in evidence if fixture_name in item.get("source", "")), None)
if match is None:
    raise SystemExit("exact-phrase search did not return the approved Markdown fixture")
evidence_id = match.get("evidence_id")
if not isinstance(evidence_id, int) or evidence_id <= 0:
    raise SystemExit("search result had no valid evidence ID")
preview = match.get("preview")
if not isinstance(preview, dict) or sys.argv[2] not in preview.get("excerpt", ""):
    raise SystemExit("granted search did not return the exact cited passage preview")
if len(preview["excerpt"].encode("utf-8")) > 512 or not isinstance(preview.get("truncated"), bool):
    raise SystemExit("preview exceeded the grant byte bound or omitted truncation status")
location = preview.get("location")
if (
    not isinstance(location, dict)
    or location.get("type") != "file"
    or location.get("path") != sys.argv[3]
    or location.get("start_line", 0) < 1
    or location.get("end_line", 0) < location["start_line"]
):
    raise SystemExit("preview lacked the approved typed Markdown source location")
print(evidence_id)
PY
)" || fail "could not validate the bounded exact-phrase search response"

interactive_json="$root/interactive-search.json"
if ! "$binary" interactive-search "${consumer_args[@]}" --limit 1 "$phrase" >"$interactive_json" 2>"$root/interactive-search.err"; then
  cat "$root/interactive-search.err" >&2
  fail "v2 interactive lexical search request failed"
fi
python3 - "$interactive_json" "$phrase" "$fixture" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
if response.get("type") != "search" or response.get("data", {}).get("query") != sys.argv[2]:
    raise SystemExit("interactive search did not return the requested query")
evidence = response["data"].get("evidence", [])
if len(evidence) != 1:
    raise SystemExit("interactive search did not honor the grant's one-result bound")
preview = evidence[0].get("preview", {})
location = preview.get("location", {})
if sys.argv[2] not in preview.get("excerpt", "") or location.get("path") != sys.argv[3]:
    raise SystemExit("interactive search did not return the approved cited Markdown passage")
PY

evidence_json="$root/consumer-evidence.json"
if ! "$binary" open-evidence "${consumer_args[@]}" --evidence-id "$evidence_id" >"$evidence_json" 2>"$root/consumer-evidence.err"; then
  cat "$root/consumer-evidence.err" >&2
  fail "granted open-evidence request failed"
fi
python3 - "$evidence_json" "$phrase" "$fixture" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
if response.get("type") != "evidence" or not isinstance(response.get("data"), dict):
    raise SystemExit("open-evidence returned an unexpected response type")
evidence = response["data"]
if sys.argv[2] not in evidence.get("excerpt", ""):
    raise SystemExit("opened evidence did not contain the exact fixture phrase")
if len(evidence["excerpt"].encode("utf-8")) > 512:
    raise SystemExit("opened evidence exceeded the grant's byte bound")
source = evidence.get("source")
if not isinstance(source, dict) or source.get("type") != "file" or source.get("path") != sys.argv[3]:
    raise SystemExit("opened evidence did not identify the approved Markdown file")
PY

docx_phrase='A coral compass guides the observatory archivist'
docx_json="$root/docx-search.json"
if ! "$binary" search "${consumer_args[@]}" --limit 1 "$docx_phrase" >"$docx_json" 2>"$root/docx-search.err"; then
  cat "$root/docx-search.err" >&2
  fail "approved DOCX paragraph search failed"
fi
docx_id="$(python3 - "$docx_json" "$approved_root/letter.docx" "$docx_phrase" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
evidence = response.get("data", {}).get("evidence", [])
match = next((item for item in evidence if item.get("preview", {}).get("location", {}).get("path") == sys.argv[2]), None)
if not match or sys.argv[3] not in match["preview"].get("excerpt", ""):
    raise SystemExit("approved DOCX paragraph did not produce a cited excerpt")
location = match["preview"]["location"]
if (
    location.get("type") != "docx_paragraph"
    or not isinstance(location.get("start_paragraph"), int)
    or location["start_paragraph"] < 1
    or location.get("end_paragraph", 0) < location["start_paragraph"]
    or not location.get("content_hash", "").startswith("sha256:")
):
    raise SystemExit("DOCX excerpt lacked typed paragraph and snapshot provenance")
print(match["evidence_id"])
PY
)" || fail "could not validate DOCX source paragraph preview"
if ! "$binary" open-evidence "${consumer_args[@]}" --evidence-id "$docx_id" >"$root/docx-evidence.json" 2>"$root/docx-evidence.err"; then
  cat "$root/docx-evidence.err" >&2
  fail "authorized DOCX paragraph evidence could not reopen"
fi
python3 - "$root/docx-evidence.json" "$docx_phrase" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    evidence = json.load(stream).get("data", {})
if sys.argv[2] not in evidence.get("excerpt", "") or evidence.get("source", {}).get("type") != "docx_paragraph":
    raise SystemExit("opened DOCX evidence did not retain typed paragraph citation")
PY

pdf_phrase='The azure narwhal guards the archive'
pdf_json="$root/pdf-search.json"
if ! "$binary" search "${consumer_args[@]}" --limit 1 "$pdf_phrase" >"$pdf_json" 2>"$root/pdf-search.err"; then
  cat "$root/pdf-search.err" >&2
  fail "approved text-bearing PDF page search failed"
fi
pdf_id="$(python3 - "$pdf_json" "$pdf_phrase" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
evidence = response.get("data", {}).get("evidence", [])
match = next((item for item in evidence if sys.argv[2] in item.get("preview", {}).get("excerpt", "")), None)
if not match:
    raise SystemExit("search did not find the approved text-bearing PDF")
preview = match.get("preview", {})
location = preview.get("location", {})
if "path" in location:
    raise SystemExit("PDF search preview exposed a source path")
valid_page = (
    location.get("type") == "pdf"
    and location.get("page_start") == 1
    and location.get("page_end") == 1
) or (
    location.get("type") == "pdf_region"
    and location.get("page") == 1
    and isinstance(location.get("width"), int)
    and isinstance(location.get("height"), int)
    and location["width"] > 0
    and location["height"] > 0
)
if sys.argv[2] not in preview.get("excerpt", "") or not valid_page:
    raise SystemExit("PDF excerpt lacked text and typed page or region citation")
print(match["evidence_id"])
PY
)" || { cat "$pdf_json" >&2; fail "could not validate text-bearing PDF page preview"; }
if ! "$binary" open-evidence "${consumer_args[@]}" --evidence-id "$pdf_id" >"$root/pdf-evidence.json" 2>"$root/pdf-evidence.err"; then
  cat "$root/pdf-evidence.err" >&2
  fail "authorized PDF page evidence could not reopen"
fi
pdf_source_path="$(realpath "$approved_root/text.pdf")"
python3 - "$root/pdf-evidence.json" "$pdf_phrase" "$pdf_source_path" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    evidence = json.load(stream).get("data", {})
source = evidence.get("source", {})
valid_page = (
    source.get("type") == "pdf"
    and source.get("page_start") == 1
    and source.get("page_end") == 1
) or (
    source.get("type") == "pdf_region"
    and source.get("page") == 1
    and isinstance(source.get("width"), int)
    and isinstance(source.get("height"), int)
    and source["width"] > 0
    and source["height"] > 0
)
if (
    sys.argv[2] not in evidence.get("excerpt", "")
    or not valid_page
    or source.get("path") != sys.argv[3]
):
    raise SystemExit("opened PDF evidence lacked its authorized source path or typed page/region citation")
PY

# Approving a later root must not expand an already issued consumer credential.
other_root="$root/other-approved-root"
other_fixture="$other_root/private-note.md"
other_phrase='The violet meteor measures a secluded atlas'
mkdir -p "$other_root"
printf '%s\n' '# Private second root' "$other_phrase." >"$other_fixture"
if ! "$binary" owner roots add --instance-dir "$instance" "$other_root" >"$root/add-root.out" 2>"$root/add-root.err"; then
  cat "$root/add-root.err" >&2
  fail "could not approve a second root while the installed daemon runs"
fi
other_realm="$(printf '%064x' 33)"
if [[ "$other_realm" == "$provider_realm" ]]; then
  other_realm="$(printf '%064x' 34)"
fi
other_credential="$root/other-grant.credential"
if ! other_grant="$("$binary" owner grant create-external \
  --instance-dir "$instance" \
  --consumer-realm "$other_realm" \
  --credential-file "$other_credential" \
  --access search-and-open-evidence \
  --max-sensitivity internal \
  --read-root "$other_root" \
  --max-results 1 \
  --max-evidence-bytes 512 \
  --expires-in-seconds 900 2>"$root/other-grant.err")"; then
  cat "$root/other-grant.err" >&2
  fail "could not grant only the newly approved second root"
fi
if [[ "$other_grant" != *"allowed_roots=[\"$other_root\"]"* ]]; then
  fail "second consumer grant did not freeze only the newly approved root"
fi
other_args=(--socket-path "$socket_path" --consumer-realm "$other_realm" --credential-file "$other_credential")
other_ready=false
for attempt in {1..120}; do
  if "$binary" indexing-status "${other_args[@]}" >"$root/other-indexing.json" 2>"$root/other-indexing.err"; then
    if python3 - "$root/other-indexing.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    status = json.load(stream).get("data", {})
if (
    status.get("approved_root_count") == 1
    and status.get("indexed_file_count") == 1
    and status.get("pending_file_count") == 0
    and status.get("scanning") is False
):
    raise SystemExit(0)
raise SystemExit(1)
PY
    then
      other_ready=true
      break
    fi
  fi
  sleep 0.1
done
if [[ "$other_ready" != true ]]; then
  cat "$root/other-indexing.json" >&2 || true
  fail "newly approved second root did not become indexed"
fi
if ! "$binary" indexing-status "${consumer_args[@]}" >"$root/old-grant-indexing.json" 2>"$root/old-grant-indexing.err"; then
  cat "$root/old-grant-indexing.err" >&2
  fail "first consumer inventory failed after approval of another root"
fi
python3 - "$root/old-grant-indexing.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    status = json.load(stream).get("data", {})
if status.get("approved_root_count") != 1 or status.get("indexed_file_count") != 3:
    raise SystemExit("first consumer inventory expanded to include later approved root")
PY

if ! "$binary" search "${other_args[@]}" --limit 1 "$other_phrase" >"$root/other-search.json" 2>"$root/other-search.err"; then
  cat "$root/other-search.err" >&2
  fail "second consumer could not search its own approved root"
fi
other_evidence_id="$(python3 - "$root/other-search.json" "$other_fixture" "$other_phrase" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
results = response.get("data", {}).get("evidence", [])
if len(results) != 1:
    raise SystemExit("second consumer did not find its one authorized passage")
result = results[0]
preview = result.get("preview") or {}
if preview.get("location", {}).get("path") != sys.argv[2] or sys.argv[3] not in preview.get("excerpt", ""):
    raise SystemExit("second consumer passage lacked its own source and excerpt")
print(result["evidence_id"])
PY
)" || fail "could not identify authentic second-root evidence"
if ! "$binary" open-evidence "${other_args[@]}" --evidence-id "$other_evidence_id" >"$root/other-open.json" 2>"$root/other-open.err"; then
  cat "$root/other-open.err" >&2
  fail "second consumer could not reopen its own authentic evidence"
fi
if ! "$binary" search "${consumer_args[@]}" --limit 1 "$other_phrase" >"$root/old-grant-search.json" 2>"$root/old-grant-search.err"; then
  cat "$root/old-grant-search.err" >&2
  fail "first consumer query failed after a later root was approved"
fi
python3 - "$root/old-grant-search.json" "$other_root" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
data = response.get("data", {})
root_prefix = sys.argv[2] + "/"
for evidence in data.get("evidence", []):
    location = (evidence.get("preview") or {}).get("location") or {}
    if evidence.get("source", "").startswith(root_prefix) or location.get("path", "").startswith(root_prefix):
        raise SystemExit("first consumer saw a cited passage from later approved root")
for path in data.get("path_results", []):
    if path.get("path", "").startswith(root_prefix):
        raise SystemExit("first consumer saw a filename from later approved root")
PY
if "$binary" open-evidence "${consumer_args[@]}" --evidence-id "$other_evidence_id" >"$root/old-grant-open.out" 2>"$root/old-grant-open.err"; then
  fail "first consumer opened authentic evidence from later approved root"
fi
if [[ -s "$root/old-grant-open.out" || "$(<"$root/old-grant-open.err")" != *"SourceNotSelected"* ]]; then
  cat "$root/old-grant-open.err" >&2
  fail "out-of-grant evidence did not return typed SourceNotSelected"
fi

# The installed search-only daemon owns one durable index. A fresh process must
# resume the same approved scope and grant without rebuilding client credentials.
if ! terminate_daemon; then
  fail "read-only owner daemon did not stop cleanly before restart"
fi
"$binary" start --instance-dir "$instance" >"$root/daemon-restarted.log" 2>&1 &
daemon_pid=$!
restart_ready=false
for attempt in {1..600}; do
  if "$binary" status "${consumer_args[@]}" >"$root/restart-status.json" 2>"$root/restart-status.err"; then
    restart_ready=true
    break
  fi
  if ! process_is_live "$daemon_pid"; then
    cat "$root/daemon-restarted.log" >&2
    fail "search-only daemon exited during restart"
  fi
  sleep 0.05
done
if [[ "$restart_ready" != true ]]; then
  cat "$root/daemon-restarted.log" >&2
  fail "existing external search grant did not resume after daemon restart"
fi
if ! "$binary" search "${consumer_args[@]}" --limit 1 "$phrase" >"$root/restart-search.json" 2>"$root/restart-search.err"; then
  cat "$root/restart-search.err" >&2
  fail "approved passage search did not resume after daemon restart"
fi
python3 - "$root/restart-search.json" "$phrase" "$fixture" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
results = response.get("data", {}).get("evidence", [])
if not any(
    sys.argv[2] in item.get("preview", {}).get("excerpt", "")
    and item.get("preview", {}).get("location", {}).get("path") == sys.argv[3]
    for item in results
):
    raise SystemExit("restarted search-only daemon lost the authorized cited passage")
PY
if ! "$binary" search "${other_args[@]}" --limit 1 "$other_phrase" >"$root/other-restart-search.json" 2>"$root/other-restart-search.err"; then
  cat "$root/other-restart-search.err" >&2
  fail "second consumer credential did not survive provider restart"
fi
python3 - "$root/other-restart-search.json" "$other_fixture" "$other_evidence_id" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
results = response.get("data", {}).get("evidence", [])
if not any(
    item.get("evidence_id") == int(sys.argv[3])
    and item.get("preview", {}).get("location", {}).get("path") == sys.argv[2]
    for item in results
):
    raise SystemExit("second-root cited passage or evidence identity changed after restart")
PY


# Warm the interactive snapshot before editing the source. An old cache entry
# must never release a hit, citation or excerpt after the approved file changes.
if ! "$binary" interactive-search "${consumer_args[@]}" --limit 1 "$phrase" >"$root/pre-change-interactive.json" 2>"$root/pre-change-interactive.err"; then
  cat "$root/pre-change-interactive.err" >&2
  fail "could not warm the restarted interactive source snapshot"
fi
printf '%s\n' '# Changed source' 'The original cited passage is no longer in this file.' >"$fixture"
if ! "$binary" interactive-search "${consumer_args[@]}" --limit 1 "$phrase" >"$root/stale-interactive.json" 2>"$root/stale-interactive.err"; then
  cat "$root/stale-interactive.err" >&2
  fail "interactive search failed after approved source changed"
fi
python3 - "$root/stale-interactive.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
if response.get("type") != "search" or response.get("data", {}).get("evidence"):
    raise SystemExit("interactive search released stale source hit after file edit")
PY
if "$binary" open-evidence "${consumer_args[@]}" --evidence-id "$evidence_id" >"$root/stale-open.stdout" 2>"$root/stale-open.err"; then
  fail "old FileSpan evidence reopened after its indexed source changed"
fi

expect_unauthorized() {
  local label="$1"
  shift
  local output="$root/$label.stdout" error="$root/$label.stderr"
  if "$binary" search "$@" --limit 1 "$phrase" >"$output" 2>"$error"; then
    fail "$label consumer search unexpectedly succeeded"
  fi
  if [[ -s "$output" || "$(<"$error")" != *"Unauthorized"* ]]; then
    cat "$error" >&2
    fail "$label consumer search did not return a typed Unauthorized error"
  fi
}
expect_unauthorized ungranted "${ungranted_args[@]}"

revoke_output="$("$binary" owner grant revoke --instance-dir "$instance" "$grant_digest" 2>"$root/grant-revoke.err")" || {
  cat "$root/grant-revoke.err" >&2
  fail "could not revoke the external grant"
}
if [[ "$revoke_output" != *"state=revoked"* ]]; then
  fail "owner grant revoke did not report the revoked state"
fi
expect_unauthorized revoked "${consumer_args[@]}"

if ! terminate_daemon; then
  fail "read-only owner daemon required forced termination instead of graceful shutdown"
fi
unavailable_output="$root/daemon-unavailable.stdout"
unavailable_error="$root/daemon-unavailable.stderr"
if "$binary" status "${consumer_args[@]}" >"$unavailable_output" 2>"$unavailable_error"; then
  fail "search client succeeded after the owner daemon stopped"
fi
if [[ -s "$unavailable_output" || "$(<"$unavailable_error")" != *"DaemonUnavailable"* ]]; then
  cat "$unavailable_error" >&2
  fail "stopped daemon did not produce a typed DaemonUnavailable error"
fi

printf 'Verified installed %s %s: bounded v2 interactive Markdown, DOCX paragraphs, typed authorized PDF pages/paths, image-only OCR-needed status, frozen per-consumer root isolation after later root approval, typed cross-root evidence denial, durable restart, changed-source search/open denial, grant denial/revocation and typed daemon shutdown\n' \
  "$PACKAGE_NAME" "$package_version"
