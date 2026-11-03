#!/usr/bin/env python3
"""Record locked Rust dependency licenses and optionally stage license notices."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess

BASE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--check', action='store_true')
parser.add_argument('--licenses', type=Path)
args = parser.parse_args()
env = os.environ | {'CARGO_HOME': os.environ.get('CARGO_HOME', str(Path.home() / '.cargo'))}
metadata = json.loads(subprocess.check_output([
    'cargo', 'metadata', '--manifest-path', str(BASE / 'Cargo.toml'),
    '--locked', '--offline', '--format-version', '1'], env=env))
packages = [p for p in metadata['packages'] if p['source']]
records = [{k: p.get(k) for k in ('name', 'version', 'license', 'source', 'repository')}
           for p in sorted(packages, key=lambda p: p['name'])]
assert all(p['license'] for p in records), 'Dependency missing license metadata'
content = json.dumps({'schema_version': 1, 'lockfile': 'Cargo.lock', 'dependencies': records}, indent=2) + '\n'
inventory = BASE / 'dependencies.json'
if args.check:
    assert inventory.read_text() == content, 'Runtime license inventory drift; regenerate explicitly'
else:
    inventory.write_text(content)
if args.licenses:
    for package in packages:
        root = Path(package['manifest_path']).parent
        target = args.licenses / (package['name'] + '-' + package['version'])
        target.mkdir(parents=True, exist_ok=True)
        for pattern in ('LICENSE*', 'COPYING*', 'UNLICENSE*', 'NOTICE*'):
            for source in root.glob(pattern):
                if source.is_file():
                    shutil.copyfile(source, target / source.name)
