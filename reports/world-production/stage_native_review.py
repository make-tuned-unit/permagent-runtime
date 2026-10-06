"""Stage and verify a locally built review app. Never launches or installs it."""
from pathlib import Path
import argparse, hashlib, json, shutil, subprocess

LANE = Path(__file__).resolve().parents[2]
TARGET = LANE.parent / '.shared-target/world-production-20261006'
BASE = Path('/Applications/Permagent.app')
STAGE = TARGET / 'staged/Permagent World Review.app'
BINARY = TARGET / 'debug/permagent-app'
REPORT = LANE / 'reports/world-production/native-package-verification.json'

def digest(path):
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()

def independent_copy(source, destination):
    """Use APFS copy-on-write when available; never hardlink a review file."""
    source, destination = Path(source), Path(destination)
    temporary = destination.with_name(destination.name + '.stage-copy')
    if temporary.exists():
        raise RuntimeError(f'Stale staging copy requires inspection: {temporary}')
    try:
        copied = subprocess.run(['cp', '-cp', str(source), str(temporary)], capture_output=True)
        if copied.returncode:
            shutil.copy2(source, temporary)
        if temporary.stat().st_ino == source.stat().st_ino or digest(temporary) != digest(source):
            raise RuntimeError(f'Independent copy verification failed: {source}')
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
    return str(destination)

def snapshot(root):
    result = {}
    for path in root.rglob('*'):
        if path.is_symlink() and not path.resolve().is_relative_to(root.resolve()):
            raise RuntimeError(f'Bundle has external symlink: {path}')
        if path.is_file():
            result[str(path.relative_to(root))] = digest(path)
    return result

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--refresh', action='store_true', help='Refresh a closed, previously verified review bundle; preserve the old executable')
args = parser.parse_args()

if not BINARY.is_file():
    raise SystemExit('New native binary missing; staging refused')
if STAGE.exists() and not args.refresh:
    raise SystemExit('Stage already exists; inspect it before replacing')
if args.refresh:
    if not STAGE.is_dir() or not REPORT.is_file():
        raise SystemExit('Refresh requires a previously verified review bundle')
    previous = json.loads(REPORT.read_text())
    staged_binary = STAGE / 'Contents/MacOS/permagent-app'
    if previous['staged_app'] != str(STAGE) or previous['staged_binary_sha256'] != digest(staged_binary):
        raise SystemExit('Existing review bundle does not match its verification ledger')
    process_commands = subprocess.check_output(['ps', '-axo', 'command='], text=True).splitlines()
    if any(command.startswith(str(staged_binary)) for command in process_commands):
        raise SystemExit('Close the review app before refreshing its executable')
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(STAGE)], check=True)
embedded = json.loads((LANE / 'reports/world-production/native-embedded-assets-verification.json').read_text())
if embedded['native_build_sha256'] != digest(BINARY) or not embedded['embedded_review_identifier']:
    raise SystemExit('Current shell embedding/identity evidence missing or stale')
before = snapshot(BASE)
STAGE.parent.mkdir(parents=True, exist_ok=True)
if args.refresh:
    backup = STAGE.parent / ('review-backups/' + previous['staged_binary_sha256'][:12])
    backup.mkdir(parents=True, exist_ok=False)
    for relative in ['Contents/MacOS/permagent-app', 'Contents/Info.plist', 'Contents/_CodeSignature/CodeResources']:
        destination = backup / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        independent_copy(STAGE / relative, destination)
    independent_copy(REPORT, backup / REPORT.name)
else:
    shutil.copytree(BASE, STAGE, symlinks=True, copy_function=independent_copy)
for relative in before:
    source, destination = BASE / relative, STAGE / relative
    if not source.is_symlink() and source.stat().st_ino == destination.stat().st_ino:
        raise RuntimeError('Staging must not hardlink installed files')
independent_copy(BINARY, STAGE / 'Contents/MacOS/permagent-app')
info = STAGE / 'Contents/Info.plist'
plist = json.loads(subprocess.check_output(['plutil','-convert','json','-o','-',str(info)]))
for key, value in dict(CFBundleIdentifier='ai.permagent.world-review-20261006',
                      CFBundleName='Permagent World Review', CFBundleDisplayName='Permagent World Review').items():
    subprocess.run(['plutil','-replace' if key in plist else '-insert',key,'-string',value,str(info)],check=True)
    plist[key] = value
subprocess.run(['codesign', '--force', '--deep', '--sign', '-', '--entitlements',
                str(LANE / 'ui/desktop/src-tauri/Entitlements.plist'), str(STAGE)], check=True)
verification = subprocess.run(['codesign', '--verify', '--deep', '--strict', '--verbose=2', str(STAGE)],
                              text=True, capture_output=True)
architecture = subprocess.check_output(['lipo', '-archs', str(STAGE / 'Contents/MacOS/permagent-app')], text=True).strip()
signature = subprocess.run(['codesign', '-d', '--verbose=4', str(STAGE)], text=True, capture_output=True)
if snapshot(BASE) != before:
    raise RuntimeError('Installed baseline changed during staging')
if verification.returncode != 0 or architecture != 'arm64':
    raise RuntimeError(f'Package verification failed: {verification.stderr}; arch={architecture}')
if digest(BINARY) == before['Contents/MacOS/permagent-app']:
    raise RuntimeError('New native shell equals old installed binary')
report = dict(staged_app=str(STAGE), native_build_binary=str(BINARY),
              native_build_sha256=digest(BINARY), staged_binary_sha256=digest(STAGE / 'Contents/MacOS/permagent-app'),
              architecture=architecture, identifier=plist['CFBundleIdentifier'],
              name=plist['CFBundleName'], installed_baseline_unchanged=True,
              installed_baseline_sha256=before, signature_check=verification.stderr.strip(),
              signature_details=signature.stderr.strip(), launched=False,
              copy='verified independent-inode copies; APFS copy-on-write when supported; no hardlinks',
              staged_files=sum(1 for path in STAGE.rglob('*') if path.is_file()),
              staged_bytes=sum(path.stat().st_size for path in STAGE.rglob('*') if path.is_file()))
REPORT.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({key:value for key,value in report.items() if key not in ['installed_baseline_sha256','signature_details']}, indent=2))
