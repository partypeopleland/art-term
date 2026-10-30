"""An isolated Linux/KVM AVD; never selects or resets an existing device."""
import json
import fcntl
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time
import uuid
import xml.etree.ElementTree as ET
from device import Device, ROOT, PACKAGE, require, wait


class LinuxEmulator(Device):
    def __init__(self, sdk, run, *, hardware_keyboard=True):
        self.sdk, self.run = Path(sdk).resolve(), Path(run).resolve()
        temporary = Path(tempfile.gettempdir()).resolve()
        owned_temporary = (self.run.is_relative_to(temporary)
                           and self.run.relative_to(temporary).parts
                           and self.run.relative_to(temporary).parts[0].startswith('art-term-'))
        require(self.run.is_relative_to((ROOT / 'target/android-regression').resolve()) or owned_temporary,
                'AVD must be under test target or an art-term temporary directory')
        self.run.mkdir(parents=True, exist_ok=False, mode=0o700)
        # Serialize owned AVD lifetimes. Checking then releasing a free TCP
        # port races another runner during avdmanager/boot, and concurrent
        # software-rendered AVDs can also stall accessibility dumps.
        lock_root = ROOT / 'target/android-regression'
        lock_root.mkdir(parents=True, exist_ok=True)
        self.lock = (lock_root / '.linux-emulator.lock').open('a')
        fcntl.flock(self.lock, fcntl.LOCK_EX)
        self.name = 'art-term-scroll-' + uuid.uuid4().hex
        self.adb = self.sdk / 'platform-tools/adb'
        self.forward_ports, self.reverse_ports = [], []
        self.process = None
        self.verified = False
        self.log = None
        self.hardware_keyboard = hardware_keyboard
        # Console and ADB ports must both be free.
        for port in range(5580, 5680, 2):
            holders = []
            try:
                for p in [port, port + 1]:
                    sock = socket.socket(); holders.append(sock); sock.bind(('127.0.0.1', p))
                self.serial = 'emulator-' + str(port)
                break
            except OSError:
                continue
            finally:
                for sock in holders: sock.close()
        else:
            raise RuntimeError('No free emulator port pair')
        self.env = dict(os.environ, ANDROID_AVD_HOME=str(self.run/'avd-index'),
                        ANDROID_USER_HOME=str(self.run/'android-user'))
        for directory in ['avd-index', 'android-user']:
            (self.run/directory).mkdir()
        self.avd = self.run/'device.avd'
        self.manifest = self.run/'owner.json'

    def start_emulator(self):
        tool = self.sdk/'emulator/emulator'
        accel = subprocess.run([str(tool), '-accel-check'], capture_output=True, timeout=15)
        require(accel.returncode == 0, 'Usable KVM required')
        manager = self.sdk/'cmdline-tools/latest/bin/avdmanager'
        result = subprocess.run([str(manager), 'create', 'avd', '--name', self.name,
                                 '--package', 'system-images;android-34;default;x86_64', '--path', str(self.avd)],
                                input=b'no\n', env=self.env, capture_output=True, timeout=90)
        (self.run/'avd-create.log').write_bytes(result.stdout + result.stderr)
        require(result.returncode == 0, 'AVD creation failed; see avd-create.log')
        config = self.avd/'config.ini'
        values = {'hw.keyboard':'yes' if self.hardware_keyboard else 'no', 'hw.lcd.width':'412', 'hw.lcd.height':'915', 'hw.lcd.density':'160'}
        text = config.read_text()
        for key, value in values.items():
            text = re.sub(r'(?m)^'+re.escape(key)+r'=.*\n?', '', text) + f'\n{key}={value}\n'
        config.write_text(text)
        self.log = (self.run/'emulator.log').open('wb')
        self.process = subprocess.Popen([str(tool), '-avd', self.name, '-port', self.serial.split('-')[1],
                                         '-no-window', '-no-snapshot', '-no-audio', '-gpu', 'swiftshader_indirect',
                                         '-memory', '2048', '-cores', '2', '-no-boot-anim'],
                                        env=self.env, stdout=self.log, stderr=subprocess.STDOUT)
        self.manifest.write_text(json.dumps({'serial': self.serial, 'avdName': self.name,
                                            'avdPath': str(self.avd), 'pid': self.process.pid}, indent=2)+'\n')
        def booted():
            require(self.process.poll() is None, 'Owned emulator exited')
            try:
                p = subprocess.run([str(self.adb), '-s', self.serial, 'shell', 'getprop', 'sys.boot_completed'],
                                   capture_output=True, timeout=5)
            except subprocess.TimeoutExpired:
                return False
            return p.returncode == 0 and p.stdout.strip() == b'1'
        wait(booted, 180)
        self.verify()
        self.call('shell', 'settings', 'put', 'secure', 'show_ime_with_hard_keyboard', '0')
        self.key(82)
        return self

    def verify(self):
        require(self.process is not None and self.process.poll() is None, 'Owned emulator not running')
        require(self.text('emu', 'avd', 'name').splitlines()[0] == self.name, 'Emulator identity mismatch')
        require(self.serial.startswith('emulator-'), 'Physical devices forbidden')
        self.verified = True

    def shutdown(self):
        try:
            if self.process and self.process.poll() is None:
                try:
                    self.verify()
                    subprocess.run([str(self.adb), '-s', self.serial, 'emu', 'kill'], capture_output=True, timeout=10)
                finally:
                    # Even if ADB is unavailable, this exact Popen belongs to
                    # us. Reap it rather than leaving an orphaned emulator.
                    try: self.process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        self.process.terminate(); self.process.wait(timeout=10)
        finally:
            if self.log: self.log.close()
            fcntl.flock(self.lock, fcntl.LOCK_UN)
            self.lock.close()

    def screenshot(self, path):
        Path(path).write_bytes(self.call('exec-out', 'screencap', '-p'))

    def ui(self):
        # Accessibility can briefly be unavailable immediately after launching
        # an Activity. Retry only this read; never retry a mutating UI action.
        for attempt in range(3):
            try:
                self.call('shell', 'uiautomator', 'dump', '/sdcard/art-term-scroll-window.xml')
                return ET.fromstring(self.call('exec-out', 'cat', '/sdcard/art-term-scroll-window.xml'))
            except (RuntimeError, ET.ParseError):
                if attempt == 2: raise
                time.sleep(.3)
