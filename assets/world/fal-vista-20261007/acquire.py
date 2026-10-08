"""Reuse the verified-HTTPS world downloader with normalized completed receipts.

Usage: python3 acquire.py candidate-a /path/to/private-completed-result.json
The input must contain status=completed, request_id, result.images[0].
Do not commit private receipts containing signed URLs.
"""
from pathlib import Path
import importlib.util, json, sys, shutil

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('world_download', HERE.parent / 'fal-world-trial-20261006/download_world.py')
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)
name, receipt_path = sys.argv[1:]
if name not in ['candidate-a', 'candidate-b', 'candidate-c', 'botanical', 'cliff-waterfall', 'floating-islet', 'cliff-waterfall-alpha', 'floating-islet-alpha']:
    raise ValueError('Unexpected output name')
receipt = json.loads(Path(receipt_path).read_text())
if receipt['status'] != 'completed':
    raise ValueError('Completed result required')
if shutil.disk_usage(HERE).free < 25 * 1024**3:
    raise RuntimeError('Preserve 25 GiB free-disk floor')
target = HERE / name
target.mkdir(exist_ok=True)
private = target / '.panorama-result-private.json'
private.write_text(json.dumps({'status':'completed','request_id':receipt['request_id'], 'result':{'image':receipt['result']['images'][0]}}))
private.chmod(0o600)
try:
    helper.HERE = target
    helper.fetch('panorama')
finally:
    private.unlink(missing_ok=True)
