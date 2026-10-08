"""Pure staging-path selection and versioned ledger checks; no filesystem mutation."""
from pathlib import Path
import re


def stage_paths(target: Path, reports: Path, record_review: bool, version: str | None = None):
    if version is None:
        if record_review:
            return target / 'staged/Permagent Record Review.app', reports / 'native-record-package-verification.json'
        return target / 'staged/Permagent World Review.app', reports / 'native-package-verification.json'
    if not record_review:
        raise ValueError('--stage-version is supported only with --record-review')
    if len(version) > 40 or not re.fullmatch(r'record-[a-z0-9]+(?:-[a-z0-9]+)*', version):
        raise ValueError('--stage-version must be a lowercase record- label using only letters, digits and single hyphens (max40)')
    stage = target / 'staged' / f'Permagent Record Review ({version}).app'
    report = reports / f'native-record-package-verification-{version}.json'
    if not target.is_absolute() or not reports.is_absolute():
        raise ValueError('Versioned staging requires absolute owned target/report roots')
    if stage.resolve() != stage or report.resolve() != report:
        raise ValueError('Versioned stage/report must be canonical paths without symlink redirects')
    return stage, report


def verify_versioned_ledger(previous: dict, version: str | None, stage: Path, review_id: str):
    if version is None:
        return
    if (previous.get('stage_version') != version or previous.get('staged_app') != str(stage)
            or previous.get('record_review') is not True or previous.get('identifier') != review_id
            or previous.get('name') != 'Permagent Record Review'):
        raise ValueError('Existing versioned Record bundle does not match its exact staging/identity ledger')
