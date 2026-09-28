#!/usr/bin/env bash
set -euo pipefail

PACKAGE_NAME=io-github-briolabs-maestria-extension-worker
LAUNCHER_PACKAGE=io-github-briolabs-maestria-launcher
SEARCH_PACKAGE=io-github-briolabs-maestria-search
WORKER_BINARY=/usr/bin/maestria-extension-worker
BUBBLEWRAP=/usr/bin/bwrap

fail() {
  echo "extension worker package smoke failed: $*" >&2
  exit 1
}

if [[ $# -ne 1 || ! -d "$1" ]]; then
  echo "usage: smoke-extension-packages.sh PACKAGE_DIRECTORY" >&2
  exit 2
fi

for tool in dpkg-deb dpkg-query bwrap python3 realpath timeout sudo; do
  command -v "$tool" >/dev/null || fail "required tool is unavailable: $tool"
done

package_dir="$(realpath "$1")"
shopt -s nullglob
packages=("$package_dir"/*.deb)
if [[ ${#packages[@]} -ne 1 ]]; then
  fail "expected one Debian package in $package_dir, found ${#packages[@]}"
fi
package="${packages[0]}"

python3 - "$package" <<'PY'
from pathlib import Path
import re
import subprocess
import sys

package = Path(sys.argv[1])
expected_package = "io-github-briolabs-maestria-extension-worker"
expected_dependencies = {"bubblewrap", "libc6", "libgcc-s1"}
independent_packages = {
    "io-github-briolabs-maestria-launcher",
    "io-github-briolabs-maestria-search",
}
relation_fields = (
    "Pre-Depends",
    "Depends",
    "Recommends",
    "Suggests",
    "Enhances",
    "Breaks",
    "Conflicts",
    "Provides",
    "Replaces",
)

try:
    output = subprocess.run(
        ["dpkg-deb", "--field", str(package)],
        check=True,
        text=True,
        capture_output=True,
    ).stdout
except (FileNotFoundError, subprocess.CalledProcessError) as error:
    raise SystemExit(f"cannot read worker Debian package metadata: {error}")

fields: dict[str, str] = {}
current: str | None = None
for line in output.splitlines():
    if line[:1].isspace():
        if current is None:
            raise SystemExit("worker package metadata has an orphan continuation line")
        fields[current] += "\n" + line[1:]
        continue
    name, separator, value = line.partition(":")
    if not separator or not name or name in fields:
        raise SystemExit(f"malformed or duplicate worker package field: {line!r}")
    current = name
    fields[name] = value.strip()

if fields.get("Package") != expected_package:
    raise SystemExit(f"unexpected worker Debian package name: {fields.get('Package')!r}")
if not fields.get("Version") or fields.get("Architecture") != "amd64":
    raise SystemExit(
        "worker Debian package must have a version and amd64 architecture; "
        f"got version={fields.get('Version')!r}, architecture={fields.get('Architecture')!r}"
    )

dependencies = fields.get("Depends", "")
names: list[str] = []
for group in (part.strip() for part in dependencies.split(",") if part.strip()):
    if "|" in group:
        raise SystemExit(f"unexpected alternative worker runtime dependency: {group}")
    match = re.fullmatch(r"([a-z0-9][a-z0-9+.-]*)(?:\s*\([^()]+\))?", group)
    if match is None:
        raise SystemExit(f"unrecognized worker runtime dependency: {group}")
    names.append(match.group(1))
if set(names) != expected_dependencies or len(names) != len(expected_dependencies):
    raise SystemExit(
        "worker Debian package must depend only on bubblewrap, libc6, and libgcc-s1; "
        f"got {dependencies or '(no Depends field)'}"
    )

violations: list[str] = []
for field in relation_fields:
    relation = fields.get(field, "")
    declared = {
        match.group(1)
        for match in re.finditer(r"(?:^|[,|]\s*)([a-z0-9][a-z0-9+.-]*)", relation)
    }
    coupled = sorted(independent_packages & declared)
    if coupled:
        violations.append(f"{field}: {', '.join(coupled)}")
if violations:
    raise SystemExit(
        "extension worker package must remain independently installable from "
        f"launcher and search ({'; '.join(violations)})"
    )
PY

installed="$(dpkg-query --show --showformat='${Status}|${Version}|${Architecture}' "$PACKAGE_NAME" 2>/dev/null || true)"
expected_installed="install ok installed|$(dpkg-deb --field "$package" Version)|$(dpkg-deb --field "$package" Architecture)"
if [[ "$installed" != "$expected_installed" ]]; then
  fail "installed worker package does not match $package: got $installed"
fi

binary="$(command -v maestria-extension-worker || true)"
if [[ "$binary" != "$WORKER_BINARY" || "$(realpath "$binary")" != "$WORKER_BINARY" ]]; then
  fail "worker executable is not the installed package binary: ${binary:-not found}"
fi
binary_owner="$(dpkg-query --search "$WORKER_BINARY" 2>/dev/null || true)"
if [[ "$binary_owner" != "$PACKAGE_NAME: $WORKER_BINARY" ]]; then
  fail "worker executable is not owned by $PACKAGE_NAME: got ${binary_owner:-no package owner}"
fi

for optional_package in "$LAUNCHER_PACKAGE" "$SEARCH_PACKAGE"; do
  optional_status="$(dpkg-query --show --showformat='${Status}' "$optional_package" 2>/dev/null || true)"
  if [[ "$optional_status" == "install ok installed" ]]; then
    fail "worker-only smoke unexpectedly found $optional_package installed"
  fi
done
for optional_binary in /usr/bin/maestria-launcher /usr/bin/maestria-search; do
  if [[ -e "$optional_binary" ]]; then
    fail "worker-only smoke unexpectedly found $optional_binary"
  fi
done

umask 077
root="$(mktemp -d -t maestria-extension-package-smoke.XXXXXX)"
worker_log="$root/worker.log"
cleanup() {
  rm -rf "$root"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ ! -x "$BUBBLEWRAP" ]]; then
  fail "bubblewrap is not installed at $BUBBLEWRAP"
fi

sandbox_args=(
  --unshare-user
  --unshare-pid
  --unshare-net
  --unshare-ipc
  --unshare-uts
  --disable-userns
  --cap-drop ALL
  --die-with-parent
  --new-session
  --clearenv
  --setenv HOME /home/extension
  --setenv LANG C.UTF-8
  --setenv PATH /usr/bin
  --ro-bind /usr /usr
  --ro-bind-try /lib /lib
  --ro-bind-try /lib64 /lib64
  --proc /proc
  --dev /dev
  --dir /home
  --size 16777216
  --tmpfs /tmp
  --dir /extension
  --chdir /extension
)
# Do not treat an installed worker as extension support unless the launcher's
# required user/pid/network namespaces actually work on this hosted runner.
if ! timeout --kill-after=2s 10s "$BUBBLEWRAP" "${sandbox_args[@]}" -- /usr/bin/true >"$root/bwrap-preflight.log" 2>&1; then
  cat "$root/bwrap-preflight.log" >&2
  fail "bubblewrap namespace preflight failed; refusing to run the worker unsandboxed"
fi

host_secret="$root/host-only-secret.js"
printf '%s\n' 'export const secret = "host filesystem must stay outside the extension sandbox";' >"$host_secret"
if ! timeout --kill-after=2s 10s "$BUBBLEWRAP" "${sandbox_args[@]}" -- /usr/bin/test ! -e "$host_secret" >"$root/bwrap-filesystem-check.log" 2>&1; then
  cat "$root/bwrap-filesystem-check.log" >&2
  fail "bubblewrap sandbox exposed a host-only filesystem canary"
fi

bundle="$root/bundle"
mkdir -p "$bundle/dist"
cat >"$bundle/manifest.json" <<'EOF'
{
  "apiVersion": 1,
  "id": "dev.sillage.package-smoke",
  "name": "Extension Package Smoke",
  "version": "1.0.0",
  "entrypoints": [{"id": "main", "file": "dist/main.js"}],
  "commands": [{"id": "sandbox-probe", "title": "Sandbox probe", "entrypointId": "main"}],
  "permissions": []
}
EOF
cat >"$bundle/dist/main.js" <<'EOF'
export default {
  commands: {
    "sandbox-probe": async () => {
      const ambientHostIo =
        typeof globalThis.process !== "undefined" ||
        typeof globalThis.require !== "undefined" ||
        typeof globalThis.Deno !== "undefined";
      return {
        kind: "detail",
        title: "Sandboxed extension package smoke",
        blocks: [
          {
            kind: "text",
            text: `ambient-host-io=${ambientHostIo ? "present" : "unavailable"}`
          }
        ]
      };
    }
  }
};
EOF

if ! timeout --kill-after=2s 25s python3 - "$BUBBLEWRAP" "$WORKER_BINARY" "$bundle" >"$root/worker-protocol.jsonl" 2>"$worker_log" <<'PY'
import json
import resource
import select
import subprocess
import sys
import time

bubblewrap, worker, bundle = sys.argv[1:]
sandbox_args = [
    "--unshare-user",
    "--unshare-pid",
    "--unshare-net",
    "--unshare-ipc",
    "--unshare-uts",
    "--disable-userns",
    "--cap-drop",
    "ALL",
    "--die-with-parent",
    "--new-session",
    "--clearenv",
    "--setenv",
    "HOME",
    "/home/extension",
    "--setenv",
    "LANG",
    "C.UTF-8",
    "--setenv",
    "PATH",
    "/usr/bin",
    "--ro-bind",
    "/usr",
    "/usr",
    "--ro-bind-try",
    "/lib",
    "/lib",
    "--ro-bind-try",
    "/lib64",
    "/lib64",
    "--proc",
    "/proc",
    "--dev",
    "/dev",
    "--dir",
    "/home",
    "--size",
    "16777216",
    "--tmpfs",
    "/tmp",
    "--chdir",
    "/extension",
]


def limits() -> None:
    for limit, cap in (
        (resource.RLIMIT_AS, 512 * 1024 * 1024),
        (resource.RLIMIT_CPU, 32),
        (resource.RLIMIT_NPROC, 4096),
        (resource.RLIMIT_NOFILE, 64),
        (resource.RLIMIT_FSIZE, 16 * 1024 * 1024),
        (resource.RLIMIT_CORE, 0),
    ):
        resource.setrlimit(limit, (cap, cap))


command = [
    bubblewrap,
    *sandbox_args,
    "--ro-bind",
    worker,
    "/worker",
    "--ro-bind",
    bundle,
    "/extension",
    "--",
    "/worker",
    "--bundle-root",
    "/extension",
    "--entrypoint-id",
    "main",
    "--timeout-ms",
    "5000",
]
process = subprocess.Popen(
    command,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    start_new_session=True,
    preexec_fn=limits,
    bufsize=0,
)


def fail(message: str) -> None:
    raise RuntimeError(message)


def read_message(deadline: float) -> dict:
    assert process.stdout is not None
    remaining = deadline - time.monotonic()
    if remaining <= 0 or not select.select([process.stdout], [], [], remaining)[0]:
        fail("worker did not return its next bounded protocol response")
    line = process.stdout.readline()
    if not line:
        fail("worker exited before returning its next protocol response")
    try:
        value = json.loads(line)
    except json.JSONDecodeError as error:
        fail(f"worker emitted invalid JSON on protocol stdout: {error}")
    if not isinstance(value, dict):
        fail("worker protocol response was not a JSON object")
    return value


def send(value: dict) -> None:
    assert process.stdin is not None
    process.stdin.write((json.dumps(value, separators=(",", ":")) + "\n").encode())
    process.stdin.flush()


deadline = time.monotonic() + 12
try:
    send({
        "protocolVersion": 1,
        "kind": "command.invoke",
        "commandId": "sandbox-probe",
        "input": {},
    })
    view_message = read_message(deadline)
    if view_message.get("kind") != "view.update":
        fail(f"expected a sandboxed command view update, got {view_message!r}")
    if view_message.get("protocolVersion") != 1 or view_message.get("commandId") != "sandbox-probe":
        fail(f"view update had the wrong protocol or command identity: {view_message!r}")
    view = view_message.get("view")
    if not isinstance(view, dict) or view.get("kind") != "detail":
        fail(f"worker did not return a detail view: {view!r}")
    blocks = view.get("blocks")
    if not isinstance(blocks, list):
        fail("worker detail view omitted its text blocks")
    texts = {block.get("text") for block in blocks if isinstance(block, dict)}
    if "ambient-host-io=unavailable" not in texts:
        fail(f"extension exposed unexpected ambient host I/O APIs: {texts!r}")

    complete = read_message(deadline)
    if complete != {
        "protocolVersion": 1,
        "kind": "command.complete",
        "commandId": "sandbox-probe",
    }:
        fail(f"worker did not complete the bounded command cleanly: {complete!r}")
    assert process.stdin is not None
    process.stdin.close()
    process.wait(timeout=max(0.1, deadline - time.monotonic()))
    assert process.stdout is not None and process.stderr is not None
    extra = process.stdout.read()
    diagnostic = process.stderr.read().decode(errors="replace")
    if process.returncode != 0:
        fail(f"sandboxed worker exited with {process.returncode}: {diagnostic.strip()}")
    if extra:
        fail(f"worker emitted unexpected trailing protocol output: {extra!r}")
    if diagnostic.strip():
        fail(f"worker wrote diagnostics despite successful completion: {diagnostic.strip()}")
except Exception:
    if process.poll() is None:
        process.kill()
    process.wait()
    raise

print("Verified an installed, package-owned extension worker returning a bounded sandboxed JavaScript command view without ambient host I/O APIs.")
PY
then
  cat "$worker_log" >&2
  fail "sandboxed extension JavaScript command did not complete successfully"
fi
cat "$root/worker-protocol.jsonl"

if ! sudo -n apt-get remove --yes --no-install-recommends "$PACKAGE_NAME" >"$root/remove-worker.log" 2>&1; then
  cat "$root/remove-worker.log" >&2
  fail "could not remove the optional worker package in the disposable CI environment"
fi
removed="$(dpkg-query --show --showformat='${Status}' "$PACKAGE_NAME" 2>/dev/null || true)"
if [[ "$removed" == "install ok installed" || -e "$WORKER_BINARY" ]]; then
  fail "optional worker package removal left its package or executable installed"
fi

printf 'Verified installed %s %s: exact independent Debian runtime dependencies, package-owned executable provenance, fail-closed bubblewrap user/pid/network sandbox, inaccessible host-filesystem canary, bounded JavaScript command response without ambient host I/O APIs, and clean optional worker removal\n' \
  "$PACKAGE_NAME" "$(dpkg-deb --field "$package" Version)"
