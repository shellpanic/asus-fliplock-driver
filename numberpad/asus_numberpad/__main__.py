import argparse
import json
import os
from pathlib import Path
import sys
import subprocess

from .discovery import probe
from . import manage


def main():
    parser = argparse.ArgumentParser(prog='asus-numberpad')
    sub = parser.add_subparsers(dest='command', required=True)
    probe_parser = sub.add_parser('probe', help='read-only exact hardware discovery')
    probe_parser.add_argument('--json', action='store_true')
    for action in ('setup', 'disable'):
        child = sub.add_parser(action)
        child.add_argument('mode', choices=('plan', 'apply'))
    for action in ('run', 'recover', 'session', '_run'):
        sub.add_parser(action)
    for action in ('_match', '_refresh'):
        child = sub.add_parser(action, help=argparse.SUPPRESS)
        child.add_argument('path')
    child = sub.add_parser('_permissions', help=argparse.SUPPRESS)
    child.add_argument('user')
    child.add_argument('action', choices=('enable', 'disable'))
    args = parser.parse_args()
    if args.command == 'probe':
        print(json.dumps(probe(), indent=2))
    elif args.command in ('setup', 'disable'):
        manage.setup(args.command == 'setup', args.mode == 'apply')
    elif args.command == '_match':
        manage.match(args.path)
    elif args.command == '_refresh':
        manage.refresh(args.path)
    elif args.command == '_permissions':
        manage.permissions(args.user, args.action == 'enable')
    elif args.command == 'recover':
        manage.recover()
    elif args.command == 'session':
        manage.session()
    elif args.command in ('run', '_run'):
        from .runtime import run
        config_home = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home() / '.config')))
        config = config_home / 'asus-numberpad/config.json'
        hold = json.loads(config.read_text()).get('hold_seconds', 1.0) if config.exists() else 1.0
        run(float(hold))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f'asus-numberpad: {error}', file=sys.stderr)
        sys.exit(1)
