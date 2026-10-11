#!/usr/bin/env bash
# Linux Host deployment of the pinned official upstream runtime.
set +x
set -euo pipefail
umask 077

readonly UPSTREAM_COMMIT=b82a36620a76c84a67bc6a7ab316cbc60a30e1a9
readonly UNIT=art-term-openab-pty.service
readonly PORT=18090
readonly HTTPS_PORT=8443
binary='' binary_hash='' serve=false linger=false info=false show_secret=false upgrade=false recover=false
desktop_config='' recover_desktop=false rollback_desktop=false
user='' root='' unit_dir='' unit_file='' parent='' dns='' state=''
scratch='' installed=false enabled=false committed=false
readonly RUNTIME_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/openab-runtime
readonly HELPER=$RUNTIME_DIR/host_bundle.py

usage() {
  cat <<'EOF'
Usage: bash scripts/init-host.sh [first-install options | existing-install options]

First install (default action): download the pinned official openab-pty binary
from its GHCR image (digest-verified, via Python 3) and start a user service on
loopback 127.0.0.1:18090. No Rust/Cargo, Docker or source patches are needed.
  --serve             Also add private Tailscale HTTPS :8443 (may request sudo).
  --enable-linger     Enable this user's service at boot (may request sudo).
  --binary PATH --sha256 HEX
                      Install a local unmodified official binary instead of
                      downloading (required on aarch64). PATH must be an
                      absolute regular file, HEX its 64-digit sha256.
  --enable-desktop PATH
                      Opt into the preview launcher; reads MCP URL/token from
                      an owner-only 0600 JSON config.

Existing installation (normal reruns keep credentials and the running service):
  --show-info         Inspect the installation without changing it.
  --show-credential   Also print its admin credential ONLY to a real terminal.
  --enable-linger     Enable boot persistence on the existing service.
  --serve             Add the private HTTPS :8443 route; fails safely on conflict.
  --enable-desktop PATH
                      Activate preview on the existing install; cannot change
                      Serve or linger.
  --upgrade-runtime   Migrate to the official bundle; ALL PTYs terminate;
                      enrollment is preserved. Optional --binary/--sha256.
                      Cannot combine with inspection, Serve, linger or recovery.
  --recover-upgrade   Reconcile a durable interrupted upgrade. Exclusive flag.
  --recover-desktop   Recover an interrupted desktop activation. Exclusive flag.
  --rollback-desktop  Disable official preview (keep empty env for existing
                      units); legacy: restore baseline. Exclusive flag.

Prerequisites: run as the Linux user whose Shell you want (never sudo bash);
Linux x86_64/aarch64 with a systemd user manager; Tailscale already connected;
Python 3 and curl. See docs/openab-host-setup.md.

No coding CLI, Tailnet login or public Funnel setup is performed.
EOF
}

die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

parse_args() {
  while (($#)); do
    case "$1" in
      --upgrade-runtime) upgrade=true;;
      --recover-upgrade) recover=true;;
      --recover-desktop) recover_desktop=true;;
      --rollback-desktop) rollback_desktop=true;;
      --enable-desktop)
        (($# >= 2)) || die "$1 needs a config path"
        desktop_config=$2; shift;;
      --serve) serve=true;;
      --enable-linger) linger=true;;
      --show-info) info=true;;
      --show-credential) show_secret=true; info=true;;
      --binary|--sha256)
        (($# >= 2)) || die "$1 needs a value"
        if [[ $1 == --binary ]]; then binary=$2; else binary_hash=$2; fi
        shift;;
      -h|--help) usage; exit 0;;
      *) die "Unknown option: $1";;
    esac
    shift
  done
}

require_complete_checkout() {
  # host_bundle.py loads official_artifact.py, desktop_bundle.py and copies the
  # launcher files; a partial checkout must fail before any host mutation.
  local missing=() name
  for name in host_bundle.py official_artifact.py preview.py mcp_preview.py desktop_bundle.py; do
    [[ -f $RUNTIME_DIR/$name ]] || missing+=("$name")
  done
  ((${#missing[@]} == 0)) || die "Incomplete checkout: scripts/openab-runtime is missing ${missing[*]}. Restore the full repository checkout; a partial copy cannot install safely."
}

preflight_checks() {
  if [[ -n $desktop_config ]]; then
    [[ $desktop_config == /* && $info == false && $upgrade == false && $recover == false && $recover_desktop == false && $rollback_desktop == false ]] || die 'Desktop activation conflicts with inspection/upgrade/recovery.'
  fi
  if [[ $recover_desktop == true || $rollback_desktop == true ]]; then
    [[ $recover_desktop != "$rollback_desktop" && -z $desktop_config && -z $binary && -z $binary_hash && $info == false && $upgrade == false && $recover == false && $serve == false && $linger == false ]] || die 'Desktop recovery/rollback must be used alone.'
  fi
  [[ $upgrade == false || ( $recover == false && $info == false && $serve == false && $linger == false ) ]] || die 'Upgrade conflicts with recovery, inspection, Serve or linger flags.'
  [[ $recover == false || ( $upgrade == false && $info == false && $serve == false && $linger == false && -z $binary && -z $binary_hash ) ]] || die 'Recovery must be used alone.'
  if [[ $serve == true || $linger == true ]]; then
    command -v sudo >/dev/null || die 'Requested setup needs sudo; see the guide.'
  fi
  [[ $(uname -s) == Linux ]] || die 'Only Linux/systemd Hosts are supported.'
  [[ $(id -u) != 0 ]] || die 'Do not use sudo bash; run as the Shell user.'
  case $(uname -m) in x86_64|aarch64) ;; *) die 'Only x86_64/aarch64 Hosts are supported.';; esac
  [[ ${HOME:-} =~ ^/[a-zA-Z0-9_./-]+$ && $HOME != / ]] || die 'HOME must be an absolute simple path without whitespace.'
  local cmd
  for cmd in python3 systemctl loginctl tailscale curl timeout flock sha256sum install; do
    command -v "$cmd" >/dev/null || die "Missing dependency: $cmd (see docs/openab-host-setup.md)"
  done
  require_complete_checkout
  if [[ -n $binary || -n $binary_hash ]]; then
    [[ $binary == /* && -f $binary && ! -L $binary && $binary_hash =~ ^[a-fA-F0-9]{64}$ ]] || die '--binary needs an absolute regular file and --sha256 with 64 hex digits.'
  elif [[ $(uname -m) != x86_64 && $info == false && $recover == false && $recover_desktop == false && $rollback_desktop == false ]]; then
    die 'The pinned official runtime artifact is x86_64-only; supply an unmodified official binary via --binary/--sha256 on this architecture.'
  fi
  [[ $info == false || ( $serve == false && $linger == false && -z $binary ) ]] || die 'Inspection cannot be combined with installation flags.'
  user=$(id -un)
  [[ $user =~ ^[a-zA-Z0-9_-]+$ ]] || die 'Unsupported user name.'
  root=$HOME/.local/share/art-term/openab-pty
  unit_dir=$HOME/.config/systemd/user
  unit_file=$unit_dir/$UNIT
  parent=${root%/*}
  # Reject symlinked installation paths, including parents, before mkdir/writing.
  python3 - "$root" "$unit_file" <<'PY'
import pathlib, sys
for value in sys.argv[1:]:
    p = pathlib.Path(value)
    for part in [p, *p.parents]:
        if part.is_symlink():
            raise SystemExit('Symlink in installation path; refusing: ' + str(part))
PY
  if [[ $info == true && ! -e $root && ! -e $unit_file && ! -e $parent/.upgrade-intent.json && ! -L $parent/.upgrade-intent.json ]]; then
    die 'Not installed yet. Run without --show-info first.'
  fi
  timeout 10 systemctl --user show-environment >/dev/null || die 'No systemd user manager. Log in as the intended user; see the guide.'
}

cleanup() {
  local status=$?
  trap - EXIT
  if [[ $installed == true && $committed == false ]]; then
    if [[ $enabled == true ]]; then timeout 15 systemctl --user disable --now "$UNIT" >/dev/null 2>&1 || true; fi
    rm -f -- "$unit_file"
    # root was absent initially and is exclusively this invocation's installation.
    rm -rf -- "$root"
    timeout 10 systemctl --user daemon-reload >/dev/null 2>&1 || true
  fi
  if [[ -n $scratch && $scratch == "$parent"/.install.* ]]; then rm -rf -- "$scratch"; fi
  exit "$status"
}

# Inspection on a never-installed Host must not create directories or a lock;
# the never-installed check already exited inside preflight_checks.
prepare_workspace() {
  if [[ $info == false || -d $parent ]]; then
    mkdir -p -- "$parent" "$unit_dir"
    [[ ! -L $parent/.installer.lock ]] || die 'Symlinked installer lock.'
    exec 9>"$parent/.installer.lock"
    flock -n 9 || die 'Another installer is running.'
  fi
}

tailnet_info() {
  timeout 10 tailscale status --json | python3 -c '
import json,re,sys
s=json.load(sys.stdin)
if s.get("BackendState") != "Running": raise SystemExit("Tailscale is not connected; run tailscale up first.")
d=s.get("Self",{}).get("DNSName","").rstrip(".")
if not re.fullmatch(r"[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)+\.ts\.net",d):
    raise SystemExit("Missing usable *.ts.net DNS name; enable MagicDNS and HTTPS.")
print(d)
'
}

verify_installation() {
  python3 "$HELPER" verify --root "$root" --unit "$unit_file"
}

dispatch_existing() {
  if [[ $recover == true ]]; then
    python3 "$HELPER" recover --root "$root" --unit "$unit_file"
    committed=true
    exit 0
  fi
  if [[ $recover_desktop == true || $rollback_desktop == true ]]; then
    local action=desktop-recover
    [[ $rollback_desktop == false ]] || action=desktop-rollback
    python3 "$HELPER" "$action" --root "$root" --unit "$unit_file"
    committed=true
    exit 0
  fi
  python3 "$HELPER" pending --root "$root" --unit "$unit_file"
  if [[ -n $desktop_config && ( -e $root || -e $unit_file ) ]]; then
    [[ $serve == false && $linger == false ]] || die 'Existing desktop activation cannot change Serve or linger.'
    python3 "$HELPER" desktop-enable --root "$root" --unit "$unit_file" --desktop-config "$desktop_config" --binary "$binary" --sha256 "${binary_hash,,}"
    committed=true
    exit 0
  fi
  if [[ $upgrade == true ]]; then
    [[ -d $root && -f $unit_file ]] || die 'Upgrade requires an existing managed installation.'
    python3 "$HELPER" upgrade --root "$root" --unit "$unit_file" --binary "$binary" --sha256 "${binary_hash,,}"
    committed=true
    exit 0
  fi
}

use_existing_install() {
  verify_installation
  [[ -z $binary ]] || die 'Already installed; --binary would replace it. Upgrade explicitly (see guide).'
  if [[ $linger == true ]]; then
    sudo loginctl enable-linger "$user"
    [[ $(loginctl show-user "$user" -p Linger --value) == yes ]] || die 'Boot persistence could not be enabled.'
  fi
}

install_fresh() {
  # Check conflicts before downloading or generating credentials.
  timeout 10 systemctl --user cat "$UNIT" >/dev/null 2>&1 && die 'An existing unit uses this name.'
  python3 - "$PORT" <<'PY'
import socket,sys
with socket.socket() as s:
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(('127.0.0.1',int(sys.argv[1])))
PY
  if [[ $linger == true ]]; then
    sudo loginctl enable-linger "$user"
  fi
  [[ $(loginctl show-user "$user" -p Linger --value) == yes ]] || die "Enable boot persistence first: sudo loginctl enable-linger $user (or pass --enable-linger)."
  scratch=$(mktemp -d "$parent/.install.XXXXXXXX")
  local desktop_args=() generated credential verifier
  [[ -z $desktop_config ]] || desktop_args=(--desktop-config "$desktop_config")
  python3 "$HELPER" build --root "$scratch" --binary "$binary" --sha256 "${binary_hash,,}" "${desktop_args[@]}"
  generated=$("$scratch/openab-pty" --generate-admin-credential)
  credential=$(printf '%s\n' "$generated" | sed -n 's/^credential: //p')
  verifier=$(printf '%s\n' "$generated" | sed -n 's/^admin_credential_hash = "\(sha256:[a-f0-9]*\)"$/\1/p')
  [[ $credential =~ ^[a-f0-9]{64}$ && $verifier =~ ^sha256:[a-f0-9]{64}$ ]] || die 'Unexpected upstream credential format.'
  [[ "sha256:$(printf '%s' "$credential" | sha256sum | cut -d ' ' -f 1)" == "$verifier" ]] || die 'Generated verifier mismatch.'
  printf '%s\n' "$credential" > "$scratch/credential"
  unset generated credential
  cat > "$scratch/shell" <<'SH'
#!/bin/bash
exec /bin/bash -i
SH
  chmod 700 "$scratch/shell"
  cat > "$scratch/config.toml" <<EOF
[pty]
enabled = true
listen = "127.0.0.1:$PORT"
tls_terminated_upstream = true
command = "$root/shell"
max_sessions = 4
absolute_session_ttl = "12h"
attach_token_ttl = "15m"
detached_idle_ttl = "8h"
scrollback_kib = 1024
scrollback_replay = false
admin_credential_hash = "$verifier"
EOF
  "$scratch/openab-pty" --validate-projection "$scratch/config.toml"
  cat > "$scratch/service" <<EOF
[Unit]
Description=art-term openab-pty Host runtime
[Service]
Type=simple
ExecStart=$root/openab-pty --config $root/config.toml
WorkingDirectory=$HOME
Environment=HOME=$HOME USER=$user SHELL=/bin/bash TERM=xterm-256color LANG=C.UTF-8
Environment=PATH=$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin
UMask=0077
NoNewPrivileges=true
LimitCORE=0
Restart=on-failure
RestartSec=5
KillMode=control-group
TimeoutStopSec=15
[Install]
WantedBy=default.target
EOF
  if [[ -n $desktop_config ]]; then
    python3 "$HELPER" desktop-stage --root "$scratch" --unit "$root" --desktop-config "$desktop_config"
  fi
  python3 "$HELPER" manifest --root "$scratch"
  mv -T -- "$scratch" "$root"; scratch=''; installed=true
  install -m 600 "$root/service" "$unit_file"
  timeout 10 systemctl --user daemon-reload
  enabled=true
  timeout 30 systemctl --user enable --now "$UNIT"
}

# Readiness does not put bearer credentials in process arguments or logs.
ready() {
  local headers code
  # Keep SIGKILL residue outside the exact-six-file managed bundle.
  headers=$(mktemp "$parent/.headers.XXXXXXXX")
  printf 'Authorization: Bearer %s\n' "$(cat "$root/credential")" > "$headers"
  code=$(curl --noproxy '*' --silent --output /dev/null --write-out '%{http_code}' --max-time 5 \
    --header "@$headers" "http://127.0.0.1:$PORT/admin/sessions") || code=000
  rm -f -- "$headers"
  [[ $code == 200 ]]
}

verify_ready() {
  if [[ $installed == true ]]; then
    for _ in {1..10}; do ready && break; sleep 1; done
  fi
  ready || die "Runtime is not ready. Inspect: journalctl --user -u $UNIT -n 50"
  local wrong
  wrong=$(curl --noproxy '*' --silent --output /dev/null --write-out '%{http_code}' --max-time 5 \
    --header 'Authorization: Bearer invalid-readiness-probe' "http://127.0.0.1:$PORT/admin/sessions") || wrong=000
  [[ $wrong == 401 ]] || die 'Unauthenticated access was not rejected.'
  timeout 10 systemctl --user is-active --quiet "$UNIT" || die 'Service is not active.'
  committed=true
}

serve_state() {
  timeout 10 tailscale serve status --json | python3 -c '
import json,sys
s=json.load(sys.stdin); port="8443"; target="http://127.0.0.1:18090"
def occupied(c):
    return port in c.get("TCP",{}) or any(k.endswith(":"+port) for k in c.get("Web",{})) or any(k.endswith(":"+port) and v for k,v in c.get("AllowFunnel",{}).items())
if any(occupied(c) for c in s.get("Foreground",{}).values()): print("conflict"); sys.exit()
web={k:v for k,v in s.get("Web",{}).items() if k.endswith(":"+port)}
funnel=any(k.endswith(":"+port) and v for k,v in s.get("AllowFunnel",{}).items())
if len(web)==1 and not funnel and s.get("TCP",{}).get(port)=={"HTTPS":True} and next(iter(web.values())).get("Handlers")=={"/":{"Proxy":target}}:
    print("managed")
elif occupied(s): print("conflict")
else: print("absent")
'
}

configure_ingress() {
  state=$(serve_state)
  if [[ $serve == true && $state == absent ]]; then
    printf 'Configuring private Tailscale HTTPS :%s…\n' "$HTTPS_PORT"
    # Only this previously absent port; no reset, no Funnel, no existing mounts.
    if ! timeout 120 sudo tailscale serve --bg --https="$HTTPS_PORT" "http://127.0.0.1:$PORT"; then
      printf 'Ingress setup failed; runtime installation is retained. Complete HTTPS consent, then rerun with --serve.\n' >&2
      exit 1
    fi
    state=$(serve_state)
  fi
  [[ $state != conflict || $serve == false ]] || die 'HTTPS :8443 belongs to another route/Funnel. Nothing was replaced; choose a separate Host or configure ingress manually.'
  [[ $serve == false || $state == managed ]] || die 'Serve did not create the expected private route. Inspect tailscale serve status; runtime is retained.'
}

print_summary() {
  printf '\nHost runtime ready (local authentication verified).\n'
  printf 'Shell user: %s\nTailnet DNS: %s\n' "$user" "$dns"
  printf 'Tailnet IPv4: '; timeout 10 tailscale ip -4
  if [[ $state == managed ]]; then
    printf 'Base URL: https://%s:%s\nTransport: openab-pty (HTTPS / WSS)\n' "$dns" "$HTTPS_PORT"
  else
    printf 'Ingress pending. Run this installer with --serve before remote connection.\n'
    if [[ $state == conflict ]]; then
      printf 'HTTPS :8443 currently belongs to another route/Funnel; existing configuration was preserved.\n'
    fi
  fi
  printf 'Admin credential: hidden; run bash scripts/init-host.sh --show-credential in your terminal.\n'
  printf 'Service: %s\nConfig: %s/config.toml\nCredential file: %s/credential (0600)\n' "$UNIT" "$root" "$root"
  printf 'Remote access/phone Shell test still required; use a current art-term client (official openab-pty transport).\n'
  if [[ $show_secret == true ]]; then
    [[ -t 1 && -t 0 ]] || die 'Credential display requires an interactive terminal; do not pipe/redirect it.'
    printf 'Admin credential (keep private): %s\n' "$(cat "$root/credential")" > /dev/tty
  fi
}

main() {
  parse_args "$@"
  preflight_checks
  trap cleanup EXIT
  prepare_workspace
  dispatch_existing
  dns=$(tailnet_info)
  if [[ -e $root || -e $unit_file ]]; then
    use_existing_install
  elif [[ $info == true ]]; then
    die 'Not installed yet. Run without --show-info first.'
  else
    install_fresh
  fi
  verify_ready
  configure_ingress
  print_summary
}

main "$@"
