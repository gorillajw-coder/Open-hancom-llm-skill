#!/usr/bin/env python3
"""Repack a public Windows release into Claude / ChatGPT / Codex local MCP kits.

Usage: python scripts/make-client-kits.py PUBLIC_RELEASE_ZIP OUT_DIR
The core binary, embedded guides, public library and license notices stay unchanged.
No network access, registration or release upload is performed.
"""
import hashlib
from pathlib import Path, PurePosixPath
import re
import sys
import tomllib
import zipfile

LIBRARY = {
    'library/manifest.toml', 'library/defaults.toml',
    'library/frames/official-letter.toml',
    'library/cards/client-report-letter.toml',
    'library/templates/official-letter.hwpx',
    'library/examples/client-report-letter-example.hwpx',
    'library/regulations/official-letter-standard.md',
}
BASE = {
    'oh.exe', 'VERSION.txt', 'PROMPT.md', 'INSTALL.md', 'CHANGELOG.md',
    'LICENSE', 'NOTICE', 'THIRD-PARTY-NOTICES',
    'THIRD-PARTY-NOTICES-rust-std.html', 'docs/LIBRARY-GUIDE.md',
    'install-claude.cmd', 'CLAUDE-SETUP.md',
}
CLIENT_FILES = {
    'claude': ['README.md', 'install.cmd'],
    'chatgpt': ['README.md', 'install.cmd', 'GUIA_ES.md', 'PROMPT_ES.txt'],
    'codex': ['README.md', 'install.cmd'],
}


def read_public(path):
    files, roots = {}, set()
    with zipfile.ZipFile(path) as z:
        if sum(i.file_size for i in z.infolist()) > 256 * 1024 * 1024:
            raise ValueError('Release exceeds 256 MiB')
        for info in z.infolist():
            p = PurePosixPath(info.filename)
            if p.is_absolute() or '..' in p.parts or '\\' in info.filename or len(p.parts) < 2:
                raise ValueError('Unsafe or unrooted ZIP path')
            roots.add(p.parts[0])
            if info.is_dir():
                continue
            if (info.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError('Symlinks are not supported')
            rel = '/'.join(p.parts[1:])
            if rel not in BASE | LIBRARY or rel in files:
                raise ValueError('Unexpected or duplicate release file: ' + rel)
            files[rel] = z.read(info)
    if len(roots) != 1 or set(files) != BASE | LIBRARY:
        raise ValueError('Use the complete public Windows release ZIP')
    text = files['VERSION.txt'].decode('utf-8')
    match = re.search(r'^Open-hancom Portable (\d+\.\d+\.\d+) \(public\)', text)
    if not match or not files['oh.exe'].startswith(b'MZ'):
        raise ValueError('Expected a public Windows executable release')
    digest = hashlib.sha256(files['oh.exe']).hexdigest()
    if not re.search(r'^oh\.exe sha256: ' + digest + r'\s*$', text, re.M):
        raise ValueError('Binary hash does not match VERSION.txt')
    manifest = tomllib.loads(files['library/manifest.toml'].decode('utf-8'))
    if manifest.get('anonymized') is not True or not manifest.get('version', '').endswith('-public'):
        raise ValueError('Expected an anonymized public library')
    entries = manifest['file']
    listed = ['library/' + row['path'] for row in entries]
    if len(set(listed)) != len(listed) or set(listed) != LIBRARY - {'library/manifest.toml'}:
        raise ValueError('Unexpected public library manifest')
    for row in entries:
        if hashlib.sha256(files['library/' + row['path']]).hexdigest() != row['sha256']:
            raise ValueError('Library hash mismatch: ' + row['path'])
    return match[1], files


def main():
    if len(sys.argv) != 3:
        raise ValueError('Usage: make-client-kits.py PUBLIC_RELEASE_ZIP OUT_DIR')
    version, base = read_public(sys.argv[1])
    source = Path(__file__).resolve().parents[1]
    source_version = tomllib.loads((source / 'Cargo.toml').read_text())['workspace']['package']['version']
    if version != source_version:
        raise ValueError('Use the public source checkout matching the release version')
    for rel in LIBRARY:
        if base[rel] != (source / rel).read_bytes():
            raise ValueError('Library differs from this approved public source: ' + rel)
    out = Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    names = [f'open-hancom-{version}-windows-x64-{client}.zip' for client in CLIENT_FILES]
    if any((out / n).exists() or (out / (n + '.sha256')).exists() for n in names):
        raise ValueError('Output exists; choose a new output folder')
    # Read all local inputs before writing an artifact.
    payloads = {}
    for client, rels in CLIENT_FILES.items():
        files = dict(base)
        if client != 'claude':
            for old in ['install-claude.cmd', 'CLAUDE-SETUP.md']:
                del files[old]
        for rel in ['clients/README.md', 'clients/_shared/install.cmd'] + [
            'clients/' + client + '/' + name for name in rels
        ]:
            raw = (source / rel).read_bytes()
            if rel.endswith('.cmd'):
                raw = raw.replace(b'\r\n', b'\n').replace(b'\n', b'\r\n')
            files[rel] = raw
        files['START-HERE.txt'] = (
            f'Open-hancom {version} / {client}\n'
            f'Read clients/{client}/README.md, then run clients/{client}/install.cmd.\n'
            'Extract the whole ZIP. No administrator privileges required.\n'
            'This public kit contains one example letter card.\n'
        ).encode('utf-8')
        files['SHA256SUMS.txt'] = ''.join(
            hashlib.sha256(data).hexdigest() + '  ' + rel + '\n'
            for rel, data in sorted(files.items())
        ).encode('utf-8')
        payloads[client] = files
    for client, files in payloads.items():
        name = f'open-hancom-{version}-windows-x64-{client}'
        dest = out / (name + '.zip')
        temporary = out / (name + '.zip.tmp')
        try:
            with zipfile.ZipFile(temporary, 'w', zipfile.ZIP_DEFLATED) as z:
                for rel, data in sorted(files.items()):
                    info = zipfile.ZipInfo(name + '/' + rel, (2026, 1, 1, 0, 0, 0))
                    info.compress_type = zipfile.ZIP_DEFLATED
                    info.external_attr = 0o644 << 16
                    z.writestr(info, data)
            temporary.rename(dest)
        finally:
            temporary.unlink(missing_ok=True)
        dest.with_suffix('.zip.sha256').write_text(
            hashlib.sha256(dest.read_bytes()).hexdigest() + '  ' + dest.name + '\n'
        )
        print(dest)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, OSError, zipfile.BadZipFile) as error:
        sys.exit(str(error))
