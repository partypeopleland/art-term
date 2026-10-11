"""Own a fresh Android AVD under target; never select a user's device."""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
ALLOWED = ROOT / 'target/android-regression'
HIDDEN = subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0


def require(value, message):
    if not value:
        raise RuntimeError(message)


def safe(path):
    raw = Path(path).absolute()
    require(raw.resolve().is_relative_to(ALLOWED.resolve()), 'AVD path outside test target')
    for p in [raw, *raw.parents]:
        require(not p.is_symlink() and not p.is_junction(), 'AVD path contains reparse point')
        if p == ROOT:
            break
    return raw


def command(args, *, env=None, timeout=30, data=None, check=True):
    p = subprocess.run([str(a) for a in args], input=data, capture_output=True,
                       timeout=timeout, env=env, creationflags=HIDDEN)
    if check:
        require(p.returncode == 0, 'Android environment command failed: ' + Path(args[0]).name)
    return p


def process_records(name):
    require(re.fullmatch(r'art-term-regression-[a-f0-9]{32}', name), 'invalid owned AVD name')
    source = ("[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); "
              "ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process | "
              "Where-Object {$_.Name -in @('emulator.exe','qemu-system-x86_64.exe','qemu-system-x86_64-headless.exe') "
              "-and $_.CommandLine -match '" + name + "'} | ForEach-Object "
              "{@{pid=$_.ProcessId;born=$_.CreationDate.ToUniversalTime().Ticks.ToString();path=$_.ExecutablePath}})")
    p = command(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', source])
    return json.loads(p.stdout.decode('utf-8-sig').strip() or '[]')


def adb(owner, *args, timeout=15, check=True):
    return command([owner['adb'], '-s', owner['serial'], *args], timeout=timeout, check=check)


def device_matches(owner):
    p = adb(owner, 'emu', 'avd', 'name', timeout=5, check=False)
    return p.returncode == 0 and p.stdout.decode(errors='replace').splitlines()[:1] == [owner['avdName']]


def save(owner, path):
    path.write_text(json.dumps(owner, indent=2) + '\n', encoding='utf-8')


def stop(manifest):
    manifest = safe(manifest)
    owner = json.loads(manifest.read_text(encoding='utf-8'))
    require(Path(owner['root']).resolve() == manifest.parent.resolve(), 'owner root mismatch')
    require(safe(owner['avdPath']).parent == manifest.parent, 'AVD directory mismatch')
    require(re.fullmatch(r'emulator-\d+', owner['serial']), 'not an emulator serial')
    current = process_records(owner['avdName'])
    known = {(p['pid'], p['born']) for p in owner.get('processes', [])}
    # The launcher can exit before first boot; a current matching unique AVD
    # process is captured during start, including on a failed boot.
    matches = [p for p in current if (p['pid'], p['born']) in known]
    if matches and device_matches(owner):
        adb(owner, 'emu', 'kill', timeout=10, check=False)
    deadline = time.monotonic() + 15
    while matches and time.monotonic() < deadline:
        time.sleep(.5)
        matches = [p for p in process_records(owner['avdName']) if (p['pid'], p['born']) in known]
    for p in matches:
        source = ("$p=Get-CimInstance Win32_Process -Filter 'ProcessId=" + str(p['pid']) + "'; "
                  "if($p -and $p.CreationDate.ToUniversalTime().Ticks.ToString() -eq '" + p['born'] +
                  "'){Stop-Process -Id " + str(p['pid']) + " -ErrorAction Stop}")
        command(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', source])
    remaining = process_records(owner['avdName'])
    require(not remaining, 'owned emulator remains running')
    owner.update(stopped=True, cleanupPassed=True)
    save(owner, manifest)
    return owner


def start():
    require(os.name == 'nt', 'Windows required for this AVD manager')
    config = json.loads((ROOT / 'target/android-tools/environment.json').read_text(encoding='utf-8-sig'))
    sdk = Path(config['androidHome'])
    emulator = sdk / 'emulator/emulator.exe'
    manager = sdk / 'cmdline-tools/latest/bin/avdmanager.bat'
    require(emulator.is_file() and manager.is_file(), 'Android emulator and cmdline-tools required')
    accel = command([emulator, '-accel-check'])
    require(b'usable' in accel.stdout.lower(), 'usable hardware acceleration required')
    require(shutil.disk_usage(ROOT).free >= 8 * 1024**3, 'at least 8 GiB free required for test AVD')
    key = uuid.uuid4().hex
    run = safe(ALLOWED / ('environment-' + key))
    run.mkdir(parents=True)
    for folder in ['android-user', 'avd-index', 'temp']:
        (run / folder).mkdir()
    env = dict(os.environ, JAVA_HOME=config['javaHome'], ANDROID_HOME=str(sdk),
               ANDROID_SDK_ROOT=str(sdk), ANDROID_USER_HOME=str(run / 'android-user'),
               ANDROID_AVD_HOME=str(run / 'avd-index'), TEMP=str(run / 'temp'), TMP=str(run / 'temp'))
    env['PATH'] = str(Path(config['javaHome']) / 'bin') + os.pathsep + env['PATH']
    name = 'art-term-regression-' + key
    avd = run / 'device.avd'
    p = command([manager, 'create', 'avd', '--name', name, '--package',
                 'system-images;android-35;google_apis;x86_64', '--path', avd],
                env=env, data=b'no\n', timeout=90)
    (run / 'avd-create.log').write_bytes(p.stdout + p.stderr)
    hardware = avd / 'config.ini'
    settings = hardware.read_text(encoding='utf-8')
    values = {'hw.ramSize': '2048', 'hw.cpu.ncore': '2', 'hw.lcd.width': '1080',
              'hw.lcd.height': '1920', 'hw.lcd.density': '420', 'hw.keyboard': 'yes',
              'disk.dataPartition.size': '2G', 'showDeviceFrame': 'no'}
    for k, v in values.items():
        settings = re.sub(r'(?m)^' + re.escape(k) + r'=.*\n?', '', settings)
        settings += '\n' + k + '=' + v + '\n'
    hardware.write_text(settings, encoding='utf-8')
    port = None
    for candidate in range(5554, 5585, 2):
        holders = []
        try:
            for number in [candidate, candidate + 1]:
                s = socket.socket()
                holders.append(s)
                s.bind(('127.0.0.1', number))
            port = candidate
            break
        except OSError:
            pass
        finally:
            for s in holders:
                s.close()
    require(port is not None, 'no free emulator console/ADB port pair')
    owner = dict(avdName=name, serial='emulator-' + str(port), avdPath=str(avd),
                 root=str(run), adb=str(sdk / 'platform-tools/adb.exe'), processes=[],
                 startedAt=datetime.datetime.now(datetime.timezone.utc).isoformat())
    manifest = run / 'owner.json'
    save(owner, manifest)
    command([owner['adb'], 'start-server'])
    with (run / 'emulator.log').open('wb') as log:
        process = subprocess.Popen([str(emulator), '-avd', name, '-port', str(port),
                                    '-no-window', '-no-snapshot', '-no-audio', '-gpu', 'software',
                                    '-memory', '2048', '-cores', '2', '-no-boot-anim'],
                                   env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=HIDDEN)
    owner['pid'] = process.pid
    try:
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            records = process_records(name)
            if records:
                owner['processes'] = records
                owner['born'] = records[0]['born']
                save(owner, manifest)
            if device_matches(owner):
                boot = adb(owner, 'shell', 'getprop', 'sys.boot_completed', check=False)
                if boot.stdout.strip() == b'1':
                    # This AVD has a hardware keyboard; text is entered through
                    # trusted CDP input. Keep the soft IME from racing touch layout.
                    adb(owner, 'shell', 'settings', 'put', 'secure', 'show_ime_with_hard_keyboard', '0')
                    keyboard = adb(owner, 'shell', 'settings', 'get', 'secure', 'show_ime_with_hard_keyboard')
                    require(keyboard.stdout.strip() == b'0', 'owned AVD keyboard setting did not persist')
                    owner['inputConfiguration'] = dict(hardwareKeyboard=True,
                                                       showImeWithHardKeyboard=False,
                                                       textInput='trusted-CDP')
                    owner['ready'] = True
                    save(owner, manifest)
                    return manifest
            require(process.poll() is None or records, 'emulator exited before ready')
            time.sleep(1)
        raise RuntimeError('owned emulator boot timed out')
    except BaseException:
        owner['processes'] = process_records(name)
        save(owner, manifest)
        stop(manifest)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['start', 'stop', 'run'])
    parser.add_argument('--owner-manifest', type=Path)
    parser.add_argument('--apk', type=Path)
    parser.add_argument('--windows-exe', type=Path)
    parser.add_argument('--edge-driver', type=Path)
    args = parser.parse_args()
    if args.action == 'stop':
        require(args.owner_manifest is not None, 'owner manifest required')
        stop(args.owner_manifest)
        print('OWNED_EMULATOR_STOPPED')
        return 0
    if args.action == 'run':
        require(args.apk and args.windows_exe and args.edge_driver, 'APK/Windows EXE/EdgeDriver required')
    manifest = start()
    owner = json.loads(manifest.read_text(encoding='utf-8'))
    print(json.dumps({'ownerManifest': str(manifest), 'serial': owner['serial']}), flush=True)
    if args.action == 'start':
        return 0
    try:
        cmd = [sys.executable, str(Path(__file__).with_name('run.py')), '--adb', owner['adb'],
               '--serial', owner['serial'], '--owner-manifest', str(manifest), '--apk', str(args.apk),
               '--windows-exe', str(args.windows_exe), '--edge-driver', str(args.edge_driver),
               '--output-root', str(ALLOWED)]
        return subprocess.run(cmd, timeout=1200, creationflags=HIDDEN).returncode
    finally:
        stop(manifest)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except Exception as error:
        print('Android environment failed: ' + str(error), file=sys.stderr)
        sys.exit(1)
