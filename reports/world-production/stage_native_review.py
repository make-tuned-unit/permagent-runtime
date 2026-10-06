"""Stage and verify a locally built review app. Never launches or installs it."""
from pathlib import Path
import hashlib, json, shutil, subprocess

LANE = Path(__file__).resolve().parents[2]
TARGET = LANE.parent / '.shared-target/world-production-20261006'
BASE = Path('/Applications/Permagent.app')
STAGE = TARGET / 'staged/Permagent World Review.app'
BINARY = TARGET / 'debug/permagent-app'
REPORT = LANE / 'reports/world-production/native-package-verification.json'

def digest(path):
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()

def snapshot(root):
    result = {}
    for path in root.rglob('*'):
        if path.is_symlink() and not path.resolve().is_relative_to(root.resolve()):
            raise RuntimeError(f'Bundle has external symlink: {path}')
        if path.is_file():
            result[str(path.relative_to(root))] = digest(path)
    return result

if not BINARY.is_file():
    raise SystemExit('New native binary missing; staging refused')
if STAGE.exists():
    raise SystemExit('Stage already exists; inspect it before replacing')
embedded = json.loads((LANE / 'reports/world-production/native-embedded-assets-verification.json').read_text())
if embedded['native_build_sha256'] != digest(BINARY) or not embedded['embedded_review_identifier']:
    raise SystemExit('Current shell embedding/identity evidence missing or stale')
before = snapshot(BASE)
STAGE.parent.mkdir(parents=True, exist_ok=True)
shutil.copytree(BASE, STAGE, symlinks=True, copy_function=shutil.copy2)
for relative in before:
    source, destination = BASE / relative, STAGE / relative
    if not source.is_symlink() and source.stat().st_ino == destination.stat().st_ino:
        raise RuntimeError('Staging must not hardlink installed files')
shutil.copy2(BINARY, STAGE / 'Contents/MacOS/permagent-app')
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
              copy='ordinary independent-inode copies; no hardlinks',
              staged_files=sum(1 for path in STAGE.rglob('*') if path.is_file()),
              staged_bytes=sum(path.stat().st_size for path in STAGE.rglob('*') if path.is_file()))
REPORT.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({key:value for key,value in report.items() if key not in ['installed_baseline_sha256','signature_details']}, indent=2))
