"""Stage and verify a locally built review app. Never launches or installs it."""
from pathlib import Path
import argparse, hashlib, json, re, shutil, subprocess, time
from native_stage_paths import stage_paths, verify_versioned_ledger

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
parser.add_argument('--record-review', action='store_true', help='Stage the distinct isolated Record review bundle; never replace its daemon')
parser.add_argument('--stage-version', help='Optional sanitized record- label for a separate owned Record bundle and verification report')
parser.add_argument('--refresh', action='store_true', help='Refresh a closed, previously verified review bundle; preserve the old executable')
parser.add_argument('--audio-helper', type=Path, help='Explicit reviewed audio helper override; never changes installed/source baseline')
parser.add_argument('--audio-helper-sha256', help='Required expected SHA-256 for --audio-helper')
args = parser.parse_args()
REVIEW_ID = 'ai.permagent.record-review-20261007' if args.record_review else 'ai.permagent.world-review-20261006'
REVIEW_NAME = 'Permagent Record Review' if args.record_review else 'Permagent World Review'
try:
    STAGE, REPORT = stage_paths(TARGET, LANE / 'reports/world-production', args.record_review, args.stage_version)
except ValueError as error:
    parser.error(str(error))
if args.record_review:
    if not args.audio_helper:
        parser.error('--record-review requires an explicit reviewed --audio-helper and --audio-helper-sha256')

if bool(args.audio_helper) != bool(args.audio_helper_sha256):
    parser.error('--audio-helper and --audio-helper-sha256 must be supplied together')
audio_override = None
if args.audio_helper:
    if not re.fullmatch(r'[0-9a-f]{64}', args.audio_helper_sha256):
        parser.error('--audio-helper-sha256 must be a lowercase 64-character SHA-256')
    helper = args.audio_helper.resolve(strict=True)
    if not helper.is_file() or helper.is_relative_to(BASE) or helper.is_relative_to(STAGE):
        raise SystemExit('Audio override must be an independent reviewed file outside installed/staged bundles')
    if digest(helper) != args.audio_helper_sha256:
        raise SystemExit('Audio helper does not match the explicit expected SHA-256')
    subprocess.run(['codesign', '--verify', '--strict', str(helper)], check=True)
    if subprocess.check_output(['lipo', '-archs', str(helper)], text=True).strip() != 'arm64':
        raise SystemExit('Audio helper must be arm64')
    audio_override = dict(source=str(helper), expected_sha256=args.audio_helper_sha256)


if not BINARY.is_file():
    raise SystemExit('New native binary missing; staging refused')
if STAGE.exists() and not args.refresh:
    raise SystemExit('Stage already exists; inspect it before replacing')
if args.stage_version is not None and REPORT.exists() and not args.refresh:
    raise SystemExit('Versioned verification ledger already exists; inspect it before replacing')
if args.refresh:
    if not STAGE.is_dir() or not REPORT.is_file():
        raise SystemExit('Refresh requires a previously verified review bundle')
    previous = json.loads(REPORT.read_text())
    try:
        verify_versioned_ledger(previous, args.stage_version, STAGE, REVIEW_ID)
    except ValueError as error:
        raise SystemExit(str(error))
    if args.stage_version is not None:
        existing_plist = json.loads(subprocess.check_output([
            'plutil', '-convert', 'json', '-o', '-', str(STAGE / 'Contents/Info.plist')]))
        if (existing_plist.get('CFBundleIdentifier') != REVIEW_ID
                or existing_plist.get('CFBundleName') != REVIEW_NAME):
            raise SystemExit('Existing versioned bundle identity differs from its compiled review identity')
    staged_binary = STAGE / 'Contents/MacOS/permagent-app'
    if previous['staged_app'] != str(STAGE) or previous['staged_binary_sha256'] != digest(staged_binary):
        raise SystemExit('Existing review bundle does not match its verification ledger')
    process_commands = subprocess.check_output(['ps', '-axo', 'command='], text=True).splitlines()
    if any(command.startswith(str(staged_binary)) for command in process_commands):
        raise SystemExit('Close the review app before refreshing its executable')
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(STAGE)], check=True)
    for relative, expected in previous.get('audio_helper_override', {}).get('staged_sha256', {}).items():
        if digest(STAGE / relative) != expected:
            raise SystemExit(f'Existing audio helper differs from its verification ledger: {relative}')
evidence_name = 'native-record-embedded-assets-verification.json' if args.record_review else 'native-embedded-assets-verification.json'
embedded = json.loads((LANE / 'reports/world-production' / evidence_name).read_text())
if embedded['native_build_sha256'] != digest(BINARY) or not embedded['embedded_review_identifier']:
    raise SystemExit('Current shell embedding/identity evidence missing or stale')
if args.record_review:
    config_path = LANE / 'reports/world-production/native-record-review-config.json'
    if (embedded.get('record_review') is not True or embedded.get('embedded_review_csp') is not True or embedded.get('identifier') != REVIEW_ID
            or embedded.get('review_config_sha256') != digest(config_path)
            or embedded.get('compiled_review_root') != str(TARGET / 'record-qa-runtime')
            or embedded.get('frontend_api_base') != 'http://127.0.0.1:3002'
            or embedded.get('frontend_production_endpoint_assets') != []):
        raise SystemExit('Isolated Record identity/root/frontend evidence is missing or stale')
    if b'PERMAGENT_AUDIOCAP_SCOPED_SOURCE_V1' not in helper.read_bytes():
        raise SystemExit('Record review helper lacks scoped audio capability')
before = snapshot(BASE)
STAGE.parent.mkdir(parents=True, exist_ok=True)
if args.refresh:
    backup = STAGE.parent / ('review-backups/' + previous['staged_binary_sha256'][:12] + '-' + str(time.time_ns()))
    backup.mkdir(parents=True, exist_ok=False)
    backup_paths = ['Contents/MacOS/permagent-app', 'Contents/Info.plist', 'Contents/_CodeSignature/CodeResources']
    backup_paths += [relative for relative in ['Contents/MacOS/permagent-audiocap', 'Contents/Resources/permagent-audiocap']
                     if (STAGE / relative).is_file()]
    for relative in backup_paths:
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
if not audio_override and args.refresh and previous.get('audio_helper_override'):
    preserved = previous['audio_helper_override']
    helper = backup / preserved['resolved_relative_path']
    audio_override = dict(source=str(helper), expected_sha256=digest(helper),
                          preserved_previous_review=True,
                          original_reviewed_source=preserved.get('original_reviewed_source', preserved['source']))
if audio_override:
    # Native resolves MacOS before Resources. Replace every existing candidate,
    # and always install the standard Tauri resource location.
    audio_paths = ['Contents/Resources/permagent-audiocap']
    if (STAGE / 'Contents/MacOS/permagent-audiocap').exists():
        audio_paths.insert(0, 'Contents/MacOS/permagent-audiocap')
    audio_override['previous_staged_sha256'] = {
        relative: digest(STAGE / relative) for relative in audio_paths if (STAGE / relative).is_file()}
    for relative in audio_paths:
        independent_copy(helper, STAGE / relative)
    audio_override['resolved_relative_path'] = audio_paths[0]
info = STAGE / 'Contents/Info.plist'
plist = json.loads(subprocess.check_output(['plutil','-convert','json','-o','-',str(info)]))
for key, value in dict(CFBundleIdentifier=REVIEW_ID,
                      CFBundleName=REVIEW_NAME, CFBundleDisplayName=REVIEW_NAME).items():
    subprocess.run(['plutil','-replace' if key in plist else '-insert',key,'-string',value,str(info)],check=True)
    plist[key] = value
subprocess.run(['codesign', '--force', '--deep', '--sign', '-', '--entitlements',
                str(LANE / 'ui/desktop/src-tauri/Entitlements.plist'), str(STAGE)], check=True)
if audio_override:
    # The recursive signing pass may rewrite nested signatures. Restore the
    # exact independently signed helper, then seal only the outer app so its
    # executable hash remains the reviewed, explicitly supplied value.
    for relative in audio_paths:
        independent_copy(helper, STAGE / relative)
        subprocess.run(['codesign', '--verify', '--strict', str(STAGE / relative)], check=True)
    subprocess.run(['codesign', '--force', '--sign', '-', '--entitlements',
                    str(LANE / 'ui/desktop/src-tauri/Entitlements.plist'), str(STAGE)], check=True)
    audio_override['staged_sha256'] = {relative: digest(STAGE / relative) for relative in audio_paths}
    if any(value != audio_override['expected_sha256'] for value in audio_override['staged_sha256'].values()):
        raise RuntimeError('Staged helper changed from explicit reviewed hash')
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
report = dict(record_review=args.record_review, staged_app=str(STAGE), native_build_binary=str(BINARY),
              native_build_sha256=digest(BINARY), staged_binary_sha256=digest(STAGE / 'Contents/MacOS/permagent-app'),
              architecture=architecture, identifier=plist['CFBundleIdentifier'],
              name=plist['CFBundleName'], installed_baseline_unchanged=True,
              installed_baseline_sha256=before, signature_check=verification.stderr.strip(),
              signature_details=signature.stderr.strip(), launched=False,
              copy='verified independent-inode copies; APFS copy-on-write when supported; no hardlinks',
              staged_files=sum(1 for path in STAGE.rglob('*') if path.is_file()),
              staged_bytes=sum(path.stat().st_size for path in STAGE.rglob('*') if path.is_file()))
if args.stage_version is not None:
    report['stage_version'] = args.stage_version
if args.record_review:
    report['daemon_lifecycle'] = 'externally managed; bundled baseline daemon is never spawned or replaced'
    report['compiled_review_root'] = embedded['compiled_review_root']
    report['frontend_api_base'] = embedded['frontend_api_base']
if audio_override:
    report['audio_helper_override'] = audio_override
if args.refresh:
    report['previous_review_backup'] = str(backup)
REPORT.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({key:value for key,value in report.items() if key not in ['installed_baseline_sha256','signature_details']}, indent=2))
