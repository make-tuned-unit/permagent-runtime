#!/usr/bin/env python3
"""Validate dependency/evidence integrity; --complete is the release gate."""
import argparse
import json
from pathlib import Path


def validate(document, base):
    nodes = document['nodes']
    by_id = {node['id']: node for node in nodes}
    if len(by_id) != len(nodes):
        raise ValueError('Duplicate node id')
    active, visited, order = set(), set(), []

    def visit(key):
        if key not in by_id:
            raise ValueError(f'Unknown dependency {key}')
        if key in active:
            raise ValueError(f'Cycle at {key}')
        if key in visited:
            return
        active.add(key)
        node = by_id[key]
        if node['status'] not in ('pending', 'in_progress', 'verified', 'blocked'):
            raise ValueError(f'Invalid status at {key}')
        if not node.get('acceptance'):
            raise ValueError(f'Missing acceptance at {key}')
        for dep in node['depends_on']:
            visit(dep)
        if node['status'] == 'verified':
            if not node.get('evidence'):
                raise ValueError(f'No verification evidence at {key}')
            for dep in node['depends_on']:
                if by_id[dep]['status'] != 'verified':
                    raise ValueError(f'{key} verified before {dep}')
        for evidence in node.get('evidence', []):
            if not (base / evidence.split('#')[0]).is_file():
                raise ValueError(f'Missing evidence {evidence} at {key}')
        active.remove(key)
        visited.add(key)
        order.append(key)

    for key in by_id:
        visit(key)
    return order


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--complete', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parent
    doc = json.loads((base / 'dag.json').read_text())
    order = validate(doc, base)
    pending = [n['id'] for n in doc['nodes'] if n['status'] != 'verified']
    print(json.dumps({'valid_dag': True, 'order': order, 'open_nodes': pending}, indent=2))
    return 1 if args.complete and pending else 0


if __name__ == '__main__':
    raise SystemExit(main())
