#!/usr/bin/env python3
"""Flag client releases newer than the versions recorded in docs/adapters/compatibility.json.

Exit 0: every watched client is at its tested version. Exit 1: a newer release exists,
so capture on it is unverified until the tests and the support table are updated.
Exit 2: a release source could not be read. Use --latest CLIENT=VERSION to supply a
version without network access (tests).
"""
import argparse, json, sys, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'docs/adapters/compatibility.json'


def fetch_json(url):
    request = urllib.request.Request(url, headers={'User-Agent': 'workspace-knowledge-release-check'})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response)


def latest(source):
    if source.startswith('npm:'):
        return fetch_json('https://registry.npmjs.org/%s/latest' % source[4:])['version']
    if source == 'cursor-update-api':
        return fetch_json('https://cursor.com/api/download?platform=darwin-arm64&releaseTrack=stable')['version']
    raise ValueError('unknown release source: ' + source)


def version_key(version):
    return [int(part) if part.isdigit() else part for part in version.replace('-', '.').split('.')]


def check(data, overrides):
    status = 0
    for adapter in data['adapters']:
        watch = adapter.get('watch')
        if not watch:
            continue
        name, tested = adapter['client'], watch['tested_version']
        try:
            found = overrides.get(name) or latest(watch['source'])
        except Exception as error:
            print('%s: could not read the latest release (%s)' % (name, type(error).__name__))
            status = max(status, 2)
            continue
        if version_key(found) > version_key(tested):
            print('%s: %s is out; capture is verified only for %s. Re-test, then update '
                  'docs/adapters/compatibility.json and rerun scripts/support-table.py.' % (name, found, tested))
            status = max(status, 1)
        else:
            print('%s: latest %s, tested %s: no drift' % (name, found, tested))
    return status


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--latest', action='append', default=[], metavar='CLIENT=VERSION')
    args = parser.parse_args(argv)
    overrides = dict(item.split('=', 1) for item in args.latest)
    return check(json.loads(SOURCE.read_text()), overrides)


if __name__ == '__main__':
    sys.exit(main())
