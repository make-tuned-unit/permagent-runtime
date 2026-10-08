"""Preserve the Cargo daemon artifact across Tauri's externalBin target copy."""
from __future__ import annotations
import hashlib
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import uuid


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def regular(path):
    if path.resolve() != path or not stat.S_ISREG(path.lstat().st_mode):
        raise RuntimeError(f'Daemon artifact must be canonical and regular: {path}')


def independent_copy(source, destination):
    # APFS clones save space, but hard links are forbidden: Tauri may truncate
    # its destination in place, including any hard-linked Cargo deps artifact.
    if destination.exists() or destination.is_symlink():
        raise RuntimeError(f'Refusing existing preservation destination: {destination}')
    if sys.platform == 'darwin':
        result = subprocess.run(['/bin/cp', '-cp', str(source), str(destination)],
                                capture_output=True)
        if result.returncode:
            destination.unlink(missing_ok=True)
            shutil.copy2(source, destination)
    else:
        shutil.copy2(source, destination)
    regular(destination)
    if os.path.samefile(source, destination):
        raise RuntimeError('Preservation copy must have an independent inode')


class DaemonPreservation:
    def __init__(self, target: Path):
        if target.resolve() != target:
            raise RuntimeError('Preservation target must be canonical')
        self.target = target
        self.path = target / 'debug/permagentd'
        self.backup = None
        self.evidence = {'path': str(self.path), 'preexisting': False, 'restored': False}

    def prepare(self):
        if not self.path.exists() and not self.path.is_symlink():
            self.evidence['status'] = 'no_preexisting_daemon'
            return
        regular(self.path)
        original = digest(self.path)
        directory = self.target / 'native-daemon-preservation'
        if directory.resolve() != directory:
            raise RuntimeError('Preservation directory must be canonical')
        directory.mkdir(mode=0o700, exist_ok=True)
        run = directory / uuid.uuid4().hex
        run.mkdir(mode=0o700)
        backup = run / 'permagentd'
        self.evidence.update(preexisting=True, original_sha256=original, backup=str(backup))
        independent_copy(self.path, backup)
        if digest(backup) != original or digest(self.path) != original:
            raise RuntimeError('Daemon changed during preservation; build must not start')
        self.backup = backup
        self.evidence['status'] = 'preserved_before_build'

    def restore(self, *, group_stopped: bool):
        if self.backup is None:
            return
        if not group_stopped:
            raise RuntimeError('Cannot restore daemon while the owned build group may still write it')
        regular(self.backup)
        expected = self.evidence['original_sha256']
        if digest(self.backup) != expected:
            raise RuntimeError('Preserved daemon hash changed; backup retained for investigation')
        if self.path.exists() or self.path.is_symlink():
            regular(self.path)
            current = digest(self.path)
        else:
            current = None
        self.evidence['after_build_sha256'] = current
        if current == expected:
            self.evidence.update(status='unchanged', restored_sha256=expected)
            return
        # Atomic replace also breaks any preexisting hard link into debug/deps.
        temp = self.path.parent / ('.permagentd-restore-' + uuid.uuid4().hex)
        try:
            independent_copy(self.backup, temp)
            if digest(temp) != expected:
                raise RuntimeError('Restoration copy failed hash verification')
            os.replace(temp, self.path)
            if digest(self.path) != expected:
                raise RuntimeError('Restored daemon failed hash verification')
        finally:
            temp.unlink(missing_ok=True)
        self.evidence.update(status='restored', restored=True, restored_sha256=expected)
