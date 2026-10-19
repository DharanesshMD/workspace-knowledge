"""Generate checksums and a dependency SPDX inventory for a curated release directory."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import uuid

ROOT=Path(__file__).resolve().parents[1]

def validate_tree(directory):
    for file in directory.rglob('*'):
        if file.is_symlink():raise ValueError('release directory must not contain symlinks')
        if file.is_file():
            name=file.relative_to(directory).as_posix()
            if '\n' in name or '\r' in name:raise ValueError('invalid artifact name')

def write_atomically(directory_fd, name, contents):
    temporary=f'.{name}.{uuid.uuid4().hex}.tmp'
    descriptor=os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644, dir_fd=directory_fd)
    try:
        with os.fdopen(descriptor, 'w', encoding='utf-8') as output:
            output.write(contents)
        # os.rename is the dir_fd-capable spelling exposed by macOS Python;
        # POSIX rename atomically replaces an existing destination.
        os.rename(temporary, name, src_dir_fd=directory_fd, dst_dir_fd=directory_fd)
    finally:
        try:os.unlink(temporary, dir_fd=directory_fd)
        except FileNotFoundError:pass

def open_release_directory(directory):
    nofollow = getattr(os, 'O_NOFOLLOW', None)
    directory_flag = getattr(os, 'O_DIRECTORY', None)
    if nofollow is None or directory_flag is None or os.open not in os.supports_dir_fd or os.rename not in os.supports_dir_fd:
        raise RuntimeError('manifest generation requires no-follow directory file access')
    return os.open(directory, os.O_RDONLY | directory_flag | nofollow)

def hash_release_file(directory_fd, relative):
    nofollow = os.O_NOFOLLOW
    directory_flag = os.O_DIRECTORY
    parent_fd=os.dup(directory_fd)
    try:
        for part in relative.parts[:-1]:
            child_fd=os.open(part, os.O_RDONLY | directory_flag | nofollow, dir_fd=parent_fd)
            os.close(parent_fd)
            parent_fd=child_fd
        descriptor=os.open(relative.name, os.O_RDONLY | nofollow, dir_fd=parent_fd)
        with os.fdopen(descriptor, 'rb') as source:
            if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
                raise ValueError('release files must be regular non-symlink files')
            digest=hashlib.sha256()
            while chunk:=source.read(1024 * 1024):digest.update(chunk)
            return digest.hexdigest()
    except OSError as error:
        raise ValueError('release files must be regular non-symlink files') from error
    finally:
        os.close(parent_fd)

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('directory',type=Path);args=parser.parse_args()
    if args.directory.is_symlink():raise ValueError('release directory must not be a symlink')
    directory=args.directory.resolve(strict=True)
    if directory==ROOT or ROOT in directory.parents and directory.name in ['target','web','.tooling']:
        raise ValueError('use a separate curated release directory')
    directory_fd=open_release_directory(directory)
    try:
        validate_tree(directory)
        version=subprocess.check_output([str(ROOT/'scripts/cargo'),'-vV'],text=True)
        host=next(line.split(': ',1)[1] for line in version.splitlines() if line.startswith('host: '))
        metadata=json.loads(subprocess.check_output([str(ROOT/'scripts/cargo'),'metadata','--locked','--offline','--format-version','1','--filter-platform',host],cwd=ROOT))
        packages=[{'SPDXID':'SPDXRef-Package-'+str(i),'name':p['name'],'versionInfo':p['version'],'downloadLocation':'NOASSERTION','filesAnalyzed':False,'licenseConcluded':'NOASSERTION','licenseDeclared':p.get('license') or 'NOASSERTION','copyrightText':'NOASSERTION'} for i,p in enumerate(metadata['packages'])]
        sbom={'spdxVersion':'SPDX-2.3','dataLicense':'CC0-1.0','SPDXID':'SPDXRef-DOCUMENT','name':'workspace-knowledge-dependency-inventory','documentNamespace':'https://spdx.org/spdxdocs/workspace-knowledge-'+str(uuid.uuid4()),'creationInfo':{'creators':['Tool: workspace-knowledge/release-manifest'],'created':datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')},'packages':packages}
        sbom_text=json.dumps(sbom,indent=2)+'\n'
        validate_tree(directory)
        hashes=[]
        for file in sorted(directory.rglob('*')):
            if file.is_symlink():raise ValueError('release directory must not contain symlinks')
            if file.is_file() and file.name not in ['SHA256SUMS','SHA256SUMS.sig','dependencies.spdx.json']:
                name=file.relative_to(directory).as_posix()
                if '\n' in name or '\r' in name:raise ValueError('invalid artifact name')
                hashes.append((name, hash_release_file(directory_fd, Path(name))))
        hashes.append(('dependencies.spdx.json', hashlib.sha256(sbom_text.encode()).hexdigest()))
        sums='\n'.join(f'{digest}  {name}' for name,digest in sorted(hashes))+'\n'
        write_atomically(directory_fd, 'dependencies.spdx.json', sbom_text)
        write_atomically(directory_fd, 'SHA256SUMS', sums)
    finally:
        os.close(directory_fd)
    print('Checksums and dependency inventory written; no signature or vulnerability attestation implied')
if __name__=='__main__':main()
